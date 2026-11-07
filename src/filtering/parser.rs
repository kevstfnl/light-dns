use hickory_proto::rr::Name;
use std::{collections::HashSet, io, net::IpAddr};

pub(super) const MAX_RULES: usize = 500_000;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
// L'ordre porte la priorité, y compris entre un domaine et ses sous-domaines.
pub(super) enum Action {
    Block,
    Allow,
    ImportantBlock,
    ImportantAllow,
    UserAllow,
}

#[derive(Eq, Hash, PartialEq)]
pub(super) struct Rule {
    pub name: Name,
    pub action: Action,
}

#[derive(Default)]
pub(super) struct Parsed {
    pub rules: HashSet<Rule>,
    pub disabled: HashSet<Rule>,
    pub ignored: usize,
}

pub(super) fn parse(text: &str, allow: bool) -> io::Result<Parsed> {
    let mut parsed = Parsed::default();
    let mut conditional_depth = 0usize;
    for line in text.trim_start_matches('\u{feff}').lines().map(str::trim) {
        match line.split_whitespace().next() {
            Some("!#if") => {
                conditional_depth += 1;
                continue;
            }
            Some("!#endif") => {
                conditional_depth = conditional_depth.saturating_sub(1);
                continue;
            }
            Some("!#include") => {
                parsed.ignored += 1;
                continue;
            }
            _ => {}
        }
        if conditional_depth != 0 {
            parsed.ignored += 1;
            continue;
        }
        if line.is_empty() || line.starts_with(['!', '#', '[']) {
            continue;
        }
        if line.len() > 1024
            || ["##", "#@#", "#$#", "#?#"]
                .iter()
                .any(|marker| line.contains(marker))
        {
            parsed.ignored += 1;
            continue;
        }
        if line.starts_with("||") || line.starts_with("@@||") {
            let (exception, body) = if let Some(body) = line.strip_prefix("@@||") {
                (true, body)
            } else {
                (false, &line[2..])
            };
            let Some((domain, options)) = body.split_once('^') else {
                parsed.ignored += 1;
                continue;
            };
            let mut important = false;
            let mut disabled = false;
            if !options.is_empty() {
                let Some(options) = options.strip_prefix('$') else {
                    parsed.ignored += 1;
                    continue;
                };
                let mut supported = true;
                for option in options.split(',') {
                    match option {
                        "important" => important = true,
                        "badfilter" => disabled = true,
                        _ => supported = false,
                    }
                }
                if !supported {
                    parsed.ignored += 1;
                    continue;
                }
            }
            let action = if allow {
                Action::UserAllow
            } else {
                match (exception, important) {
                    (false, false) => Action::Block,
                    (true, false) => Action::Allow,
                    (false, true) => Action::ImportantBlock,
                    (true, true) => Action::ImportantAllow,
                }
            };
            insert(&mut parsed, domain, action, disabled);
        } else {
            let body = line
                .split_once('#')
                .filter(|(body, _)| body.ends_with(char::is_whitespace))
                .map_or(line, |(body, _)| body);
            let mut fields = body.split_whitespace();
            let Some(first) = fields.next() else {
                continue;
            };
            let action = if allow {
                Action::UserAllow
            } else {
                Action::Block
            };
            if let Ok(ip) = first.parse::<IpAddr>() {
                let ip = ip.to_canonical();
                if !ip.is_unspecified() && !ip.is_loopback() {
                    parsed.ignored += 1;
                    continue;
                }
                for host in fields {
                    insert(&mut parsed, host, action, false);
                }
            } else if fields.next().is_none() {
                insert(&mut parsed, first, action, false);
            } else {
                parsed.ignored += 1;
            }
        }
        if parsed.rules.len() + parsed.disabled.len() > MAX_RULES {
            return Err(io::Error::other(
                "trop de règles dans une liste de filtrage",
            ));
        }
    }
    Ok(parsed)
}

fn insert(parsed: &mut Parsed, value: &str, action: Action, disabled: bool) {
    let Some(name) = domain(value) else {
        parsed.ignored += 1;
        return;
    };
    let rule = Rule { name, action };
    if disabled {
        parsed.disabled.insert(rule);
    } else {
        parsed.rules.insert(rule);
    }
}

fn domain(value: &str) -> Option<Name> {
    if !value.trim_end_matches('.').contains('.')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || value.parse::<IpAddr>().is_ok()
    {
        return None;
    }
    if value
        .trim_end_matches('.')
        .eq_ignore_ascii_case("localhost.localdomain")
    {
        return None;
    }
    let mut name = Name::from_ascii(value).ok()?.to_lowercase();
    name.set_fqdn(true);
    Some(name)
}
