use super::{SharedFilters, download, index::FilterIndex, parser};
use crate::config::{FilterSource, FilteringConfig};
use std::{
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

struct FileList {
    path: PathBuf,
    allow: bool,
    text: String,
}
struct RemoteList {
    source: FilterSource,
    text: Option<String>,
    fresh_until: Instant,
    cache_dirty: bool,
}

pub(crate) struct Updater {
    config: FilteringConfig,
    snapshot: SharedFilters,
    files: Vec<FileList>,
    remotes: Vec<RemoteList>,
    client: reqwest::Client,
}

impl Updater {
    pub(super) async fn prepare(
        config: FilteringConfig,
        snapshot: SharedFilters,
    ) -> io::Result<Self> {
        let mut files = Vec::new();
        let mut total = 0usize;
        for (paths, allow) in [(&config.blocklists, false), (&config.allowlist_files, true)] {
            for path in paths {
                let text = download::read(path, config.max_list_bytes)
                    .await
                    .map_err(|error| {
                        io::Error::new(
                            error.kind(),
                            format!("lecture de {} : {error}", path.display()),
                        )
                    })?;
                total += text.len();
                check_total(total)?;
                files.push(FileList {
                    path: path.clone(),
                    allow,
                    text,
                });
            }
        }
        let mut remotes = Vec::new();
        for source in &config.sources {
            let path = config.cache_dir.join(format!("{}.txt", source.name));
            let mut remote = RemoteList {
                source: source.clone(),
                text: None,
                fresh_until: Instant::now(),
                cache_dirty: false,
            };
            if let Ok(cached) = download::read(&path, config.max_list_bytes + 4096).await {
                let header = format!("! light-dns source {}\n", source.url);
                if let Some(text) = cached
                    .strip_prefix(&header)
                    .filter(|text| text.len() <= config.max_list_bytes)
                {
                    if let Ok(text) = validate_remote(text.to_owned()).await {
                        total += text.len();
                        check_total(total)?;
                        remote.text = Some(text);
                        if let Ok(metadata) = tokio::fs::metadata(&path).await {
                            let elapsed = metadata
                                .modified()
                                .ok()
                                .and_then(|modified| modified.elapsed().ok())
                                .unwrap_or(config.update_interval);
                            remote.fresh_until =
                                Instant::now() + config.update_interval.saturating_sub(elapsed);
                        }
                    } else {
                        tracing::warn!(source = %source.name, "cache de filtres invalide, téléchargement nécessaire");
                    }
                }
            }
            if remote.text.is_none() {
                tracing::warn!(source = %source.name, "aucun cache valide ; cette liste sera active après son téléchargement");
            }
            remotes.push(remote);
        }
        let mut updater = Self {
            client: download::client(&config)?,
            config,
            snapshot,
            files,
            remotes,
        };
        updater.publish().await?;
        Ok(updater)
    }

    pub(crate) async fn run(mut self) {
        let mut dirty = false;
        loop {
            let mut failed = false;
            let mut changed = false;
            let mut total = self.files.iter().map(|file| file.text.len()).sum::<usize>()
                + self
                    .remotes
                    .iter()
                    .filter_map(|remote| remote.text.as_ref())
                    .map(String::len)
                    .sum::<usize>();
            for file in &mut self.files {
                match download::read(&file.path, self.config.max_list_bytes).await {
                    Ok(text) => {
                        let next_total = total - file.text.len() + text.len();
                        if check_total(next_total).is_err() {
                            failed = true;
                            tracing::warn!(path = %file.path.display(), "mise à jour ignorée : limite totale de 64 Mio");
                            continue;
                        }
                        total = next_total;
                        changed |= text != file.text;
                        file.text = text;
                    }
                    Err(error) => {
                        failed = true;
                        tracing::warn!(path = %file.path.display(), %error, "ancienne liste locale conservée");
                    }
                }
            }
            for remote in &mut self.remotes {
                if Instant::now() < remote.fresh_until {
                    continue;
                }
                let result = async {
                    let text = download::fetch(
                        &self.client,
                        &remote.source.url,
                        self.config.max_list_bytes,
                    )
                    .await?;
                    validate_remote(text).await
                }
                .await;
                match result {
                    Ok(text) => {
                        let next_total =
                            total - remote.text.as_ref().map_or(0, String::len) + text.len();
                        if check_total(next_total).is_err() {
                            failed = true;
                            tracing::warn!(source = %remote.source.name, "mise à jour ignorée : limite totale de 64 Mio");
                            continue;
                        }
                        total = next_total;
                        remote.cache_dirty = true;
                        changed |= remote.text.as_ref() != Some(&text);
                        remote.text = Some(text);
                        remote.fresh_until = Instant::now() + self.config.update_interval;
                    }
                    Err(error) => {
                        failed = true;
                        tracing::warn!(source = %remote.source.name, %error, "mise à jour impossible ; dernière liste valide conservée");
                    }
                }
            }
            dirty |= changed || self.remotes.iter().any(|remote| remote.cache_dirty);
            if dirty {
                match self.publish().await {
                    Ok(()) => dirty = false,
                    Err(error) => {
                        failed = true;
                        tracing::warn!(%error, "mise à jour des filtres rejetée ; ancien index conservé");
                    }
                }
            }
            // Une liste valide reste active même si sa sauvegarde échoue, mais
            // sa persistance doit être réessayée sans attendre les prochaines 24 h.
            failed |= self.remotes.iter().any(|remote| remote.cache_dirty);
            let delay = if failed {
                self.config.update_interval.min(Duration::from_secs(300))
            } else {
                self.config.update_interval
            };
            tokio::time::sleep(delay).await;
        }
    }

    async fn publish(&mut self) -> io::Result<()> {
        let mut total = 0usize;
        let mut texts = Vec::new();
        for (text, allow) in self
            .files
            .iter()
            .map(|file| (&file.text, file.allow))
            .chain(
                self.remotes
                    .iter()
                    .filter_map(|remote| remote.text.as_ref().map(|text| (text, false))),
            )
        {
            total += text.len();
            if total > MAX_TOTAL_BYTES {
                return Err(io::Error::other("les listes dépassent 64 Mio au total"));
            }
            texts.push((text.clone(), allow));
        }
        let allowlist = self.config.allowlist.clone();
        let index = tokio::task::spawn_blocking(move || FilterIndex::compile(&texts, &allowlist))
            .await
            .map_err(io::Error::other)??;
        // Le cache n'est remplacé qu'après validation de l'ensemble des listes.
        for remote in &mut self.remotes {
            if !remote.cache_dirty {
                continue;
            }
            if let Some(text) = &remote.text {
                match download::save(
                    &self.config.cache_dir,
                    &remote.source.name,
                    &remote.source.url,
                    text,
                )
                .await
                {
                    Ok(()) => remote.cache_dirty = false,
                    Err(error) => {
                        tracing::warn!(source = %remote.source.name, %error, "échec de sauvegarde du cache de filtres")
                    }
                }
            }
        }
        tracing::info!(rules = index.len(), "index de filtrage chargé");
        self.snapshot.store(Arc::new(index));
        Ok(())
    }
}

async fn validate_remote(text: String) -> io::Result<String> {
    tokio::task::spawn_blocking(move || {
        if text
            .trim_start_matches('\u{feff}')
            .trim_start()
            .starts_with('<')
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "page HTML reçue à la place d'une liste DNS",
            ));
        }
        let parsed = parser::parse(&text, false)?;
        if parsed.rules.is_empty() && parsed.simple.is_empty() && parsed.disabled.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "aucune règle DNS compatible dans la liste distante",
            ));
        }
        Ok(text)
    })
    .await
    .map_err(io::Error::other)?
}

fn check_total(bytes: usize) -> io::Result<()> {
    if bytes > MAX_TOTAL_BYTES {
        return Err(io::Error::other("les listes dépassent 64 Mio au total"));
    }
    Ok(())
}
