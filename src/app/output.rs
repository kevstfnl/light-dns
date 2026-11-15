//! Présentation CLI uniquement : aucune collecte ni modification dans le chemin DNS.
use super::cli::{Command, Format};
use std::{
    fmt::Write,
    io::{self, IsTerminal},
    path::Path,
};
use toml::Table;

pub(super) struct Output {
    terminal: bool,
    color: bool,
}

impl Output {
    pub fn detect(no_color: bool) -> Self {
        let terminal = io::stdout().is_terminal();
        Self {
            terminal,
            color: terminal
                && !no_color
                && std::env::var_os("NO_COLOR").is_none()
                && std::env::var("TERM").is_ok_and(|term| term != "dumb"),
        }
    }

    fn paint(&self, text: &str, code: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    pub fn help(&self, text: &str) -> String {
        text.lines()
            .map(|line| {
                if line.starts_with("light-dns  ") || line.ends_with(':') {
                    self.paint(line, "1;36")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    pub fn version(&self) -> String {
        let version = format!("light-dns {}", env!("CARGO_PKG_VERSION"));
        if self.terminal {
            format!(
                "{}\n  Plateforme  {} / {}\n",
                self.paint(&version, "1;36"),
                std::env::consts::OS,
                std::env::consts::ARCH
            )
        } else {
            version + "\n"
        }
    }

    pub fn render(
        &self,
        command: &Command,
        format: Format,
        body: &str,
        socket: &Path,
    ) -> io::Result<String> {
        let structured = matches!(command, Command::Stats | Command::CacheStatus);
        if structured && (format == Format::Toml || (format == Format::Auto && !self.terminal)) {
            return Ok(body.to_owned());
        }
        if !structured {
            if !self.terminal {
                return Ok(body.to_owned());
            }
            let detail = match command {
                Command::Health(false) => "DNS local vérifié en UDP et TCP",
                Command::Health(true) => "DNS local UDP/TCP et résolution upstream vérifiés",
                Command::Reload => "Configuration rechargée ; cache DNS remplacé",
                Command::ClearCache => "Cache DNS vidé",
                _ => body.trim_end(),
            };
            return Ok(format!(
                "{}  {detail}\n  Socket  {}\n",
                self.paint("OK", "1;32"),
                clean(&socket.display().to_string())
            ));
        }
        let values: Table = toml::from_str(body).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("statistiques TOML invalides : {error}"),
            )
        })?;
        let mut out = String::new();
        writeln!(
            out,
            "{}",
            self.paint(
                if matches!(command, Command::Stats) {
                    "light-dns / Statistiques"
                } else {
                    "light-dns / Cache DNS"
                },
                "1;36"
            )
        )
        .map_err(io::Error::other)?;
        row(
            &mut out,
            "Socket de contrôle",
            &clean(&socket.display().to_string()),
        );
        if matches!(command, Command::Stats) {
            self.stats(&mut out, &values);
        } else {
            self.cache(&mut out, &values);
        }
        Ok(out)
    }

    fn section(&self, out: &mut String, name: &str) {
        out.push_str(&format!("\n{}\n", self.paint(name, "1")));
    }

    fn note(&self, out: &mut String, message: &str) {
        out.push_str(&format!("\n  {}\n  ", self.paint("À vérifier :", "1;33")));
        let mut width = 2;
        for word in message.split_whitespace() {
            let length = word.chars().count();
            if width > 2 && width + 1 + length > 78 {
                out.push_str("\n  ");
                width = 2;
            }
            if width > 2 {
                out.push(' ');
                width += 1;
            }
            out.push_str(word);
            width += length;
        }
        out.push('\n');
    }

    fn stats(&self, out: &mut String, values: &Table) {
        self.section(out, "Serveur");
        row(
            out,
            "Durée de fonctionnement",
            &num(values, "uptime_secs")
                .map(duration)
                .unwrap_or_else(unavailable),
        );
        if let Some(process) = values.get("process").and_then(|value| value.as_table()) {
            for (label, key) in [
                ("Version du serveur", "version"),
                ("Système", "os"),
                ("Architecture", "arch"),
            ] {
                row(
                    out,
                    label,
                    &process
                        .get(key)
                        .and_then(|v| v.as_str())
                        .map(clean)
                        .unwrap_or_else(unavailable),
                );
            }
            row(
                out,
                "PID serveur",
                &num(process, "pid")
                    .map(|pid| pid.to_string())
                    .unwrap_or_else(unavailable),
            );
            count_row(out, process, "Threads", "threads");
            self.section(out, "Mémoire du processus serveur");
            for (label, key) in [
                ("RSS actuelle", "rss_bytes"),
                ("Pic RSS depuis démarrage", "peak_rss_bytes"),
                ("Données virtuelles VmData", "data_bytes"),
            ] {
                bytes_row(out, process, label, key);
            }
            if let Some(error) = process.get("measurement_error").and_then(|v| v.as_str()) {
                self.note(out, &clean(error));
            }
        } else {
            row(out, "Mémoire du serveur", "indisponible (ancien serveur)");
        }
        self.section(out, "Trafic DNS / compteurs cumulés");
        for (label, key) in [
            ("Requêtes reçues", "requests"),
            ("Réponses locales", "local_answers"),
            ("Requêtes transmises", "forwarded"),
            ("Requêtes bloquées", "blocked"),
            ("CNAME bloqués", "cname_blocked"),
            ("Réécritures des filtres", "rewritten"),
            ("Réponses depuis le cache", "cache_hits"),
            ("Erreurs DNS", "errors"),
        ] {
            count_row(out, values, label, key);
        }
        row(
            out,
            "Part des requêtes bloquées",
            &ratio(num(values, "blocked"), num(values, "requests")),
        );
        row(
            out,
            "Cache / requêtes reçues",
            &ratio(num(values, "cache_hits"), num(values, "requests")),
        );
        row(
            out,
            "Débit moyen depuis démarrage",
            &match (num(values, "requests"), num(values, "uptime_secs")) {
                (Some(n), Some(t)) if t > 0 => format!("{:.2} req/s", n as f64 / t as f64),
                _ => unavailable(),
            },
        );
        self.section(out, "Upstreams / compteurs cumulés");
        for (label, key) in [
            ("Tentatives échouées", "upstream_failures"),
            ("Upstreams évités", "upstream_skipped"),
            ("Sondes effectuées", "upstream_probes"),
        ] {
            count_row(out, values, label, key);
        }
        self.section(out, "Filtrage et cache DNS");
        count_row(out, values, "Règles de filtrage actives", "filter_rules");
        count_row(out, values, "Entrées du cache", "cache_entries");
        bytes_row(
            out,
            values,
            "Poids estimé du cache",
            "cache_estimated_bytes",
        );
        count_row(out, values, "Rechargements TOML", "reloads");
        count_row(out, values, "Vidages explicites du cache", "cache_clears");
        if num(values, "errors").is_some_and(|n| n > 0) {
            self.note(out,"erreurs DNS enregistrées depuis le démarrage. Tester health, puis consulter les journaux.");
        }
        if num(values, "upstream_failures").is_some_and(|n| n > 0) {
            self.note(out,"échecs upstream cumulés ; ils ne prouvent pas une panne actuelle. Tester health --upstream.");
        }
        if num(values, "filter_rules") == Some(0) {
            out.push_str(
                "\n  Aucune règle active : filtrage désactivé ou listes absentes/en attente.\n",
            );
        }
        out.push_str("\n  Les compteurs sont cumulés et indicatifs sous concurrence.\n  Le poids du cache ne représente pas la RAM totale.\n  reloads exclut les mises à jour automatiques des listes.\n");
    }

    fn cache(&self, out: &mut String, values: &Table) {
        self.section(out, "Occupation de la génération active");
        row(
            out,
            "Cache DNS",
            match values.get("enabled").and_then(|v| v.as_bool()) {
                Some(true) => "activé",
                Some(false) => "désactivé",
                None => "indisponible",
            },
        );
        for (label, key) in [
            ("Entrées présentes", "entries"),
            ("Limite d’entrées", "max_entries"),
        ] {
            count_row(out, values, label, key);
        }
        for (label, key) in [
            ("Poids estimé", "estimated_bytes"),
            ("Budget configuré", "max_memory_bytes"),
        ] {
            bytes_row(out, values, label, key);
        }
        row(
            out,
            "Budget utilisé (estimation)",
            &ratio(
                num(values, "estimated_bytes"),
                num(values, "max_memory_bytes"),
            ),
        );
        row(
            out,
            "TTL maximal",
            &num(values, "max_ttl_secs")
                .map(duration)
                .unwrap_or_else(unavailable),
        );
        self.section(out, "Compteurs depuis démarrage");
        count_row(out, values, "Réponses depuis le cache", "hits");
        count_row(out, values, "Vidages explicites", "clears");
        if values.get("enabled").and_then(|v| v.as_bool()) == Some(true) {
            if num(values, "max_entries") == Some(0)
                || num(values, "max_memory_bytes") == Some(0)
                || num(values, "max_ttl_secs") == Some(0)
            {
                self.note(out,"cache configuré actif mais capacité ou TTL nul : aucune réponse ne sera conservée.");
            } else if let (Some(used), Some(max)) = (
                num(values, "estimated_bytes"),
                num(values, "max_memory_bytes"),
            ) && max > 0
                && (used as f64 / max as f64) >= 0.9
            {
                self.note(out,"budget du cache utilisé à au moins 90 %. Des évictions sont normales à la limite ; ce n’est pas une fuite mémoire.");
            }
        }
        out.push_str("\n  Occupation indicative après maintenance interne.\n  Le budget du cache ne plafonne pas le RSS du serveur.\n");
    }
}

fn unavailable() -> String {
    "indisponible".into()
}
fn num(values: &Table, key: &str) -> Option<u64> {
    values.get(key)?.as_integer()?.try_into().ok()
}
fn count(value: u64) -> String {
    let raw = value.to_string();
    raw.chars()
        .enumerate()
        .fold(String::new(), |mut out, (i, c)| {
            if i > 0 && (raw.len() - i).is_multiple_of(3) {
                out.push(' ');
            }
            out.push(c);
            out
        })
}
fn row(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("  {label:<30}  {value}\n"));
}
fn count_row(out: &mut String, values: &Table, label: &str, key: &str) {
    row(
        out,
        label,
        &num(values, key).map(count).unwrap_or_else(unavailable),
    );
}
fn bytes_row(out: &mut String, values: &Table, label: &str, key: &str) {
    row(
        out,
        label,
        &num(values, key).map(bytes).unwrap_or_else(unavailable),
    );
}
fn bytes(value: u64) -> String {
    let (divisor, unit) = if value >= 1024 * 1024 * 1024 {
        (1024.0 * 1024.0 * 1024.0, "Gio")
    } else if value >= 1024 * 1024 {
        (1024.0 * 1024.0, "Mio")
    } else if value >= 1024 {
        (1024.0, "Kio")
    } else {
        return format!("{} octets", count(value));
    };
    format!(
        "{:.2} {unit} ({} octets)",
        value as f64 / divisor,
        count(value)
    )
}
fn duration(seconds: u64) -> String {
    format!(
        "{} j {:02} h {:02} min {:02} s",
        seconds / 86400,
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}
fn ratio(numerator: Option<u64>, denominator: Option<u64>) -> String {
    match (numerator, denominator) {
        (Some(n), Some(d)) if d > 0 => format!("{:.1} %", n as f64 / d as f64 * 100.0),
        _ => "— (aucune base de calcul)".into(),
    }
}
// No terminal escape or line injection from a configured path or daemon text.
fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
#[path = "../../tests/app/output.rs"]
mod tests;
