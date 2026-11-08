use super::rewrite::{self, Rewrite};
use hickory_proto::rr::{Name, RecordType};
use std::{
    collections::{HashMap, HashSet},
    io,
    net::IpAddr,
    str::FromStr,
};

pub(super) const MAX_RULES: usize = 500_000;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(super) enum Action {
    Block,
    Allow,
    ImportantBlock,
    ImportantAllow,
    UserAllow,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub(super) enum Selector {
    Suffix(Name),
    Mask(String),
}

#[derive(Eq, Hash, PartialEq)]
pub(super) struct SimpleRule {
    pub name: Name,
    pub action: Action,
}

#[derive(Clone, Eq, Hash, PartialEq)]
pub(super) struct Rule {
    pub selector: Selector,
    pub action: Action,
    // Les domaines simples ne paient ni les Vec ni le RData des règles avancées.
    pub conditions: Option<Box<Conditions>>,
}

#[derive(Clone, Default, Eq, Hash, PartialEq)]
pub(super) struct Conditions {
    pub types: Vec<RecordType>,
    pub exclude_types: bool,
    pub denyallow: Vec<Name>,
    pub rewrite: Option<Rewrite>,
}

impl Rule {
    pub fn into_simple(self) -> Result<SimpleRule, Self> {
        match self {
            Self {
                selector: Selector::Suffix(name),
                action,
                conditions: None,
            } => Ok(SimpleRule { name, action }),
            advanced => Err(advanced),
        }
    }

    pub fn simple(name: Name, action: Action) -> Self {
        Self {
            selector: Selector::Suffix(name),
            action,
            conditions: None,
        }
    }
    pub fn rewrite(&self) -> Option<&Rewrite> {
        self.conditions.as_ref()?.rewrite.as_ref()
    }
    pub fn applies(&self, name: &Name, kind: RecordType) -> bool {
        let Some(conditions) = &self.conditions else {
            return true;
        };
        (conditions.types.is_empty()
            || conditions.types.contains(&kind) != conditions.exclude_types)
            && !conditions
                .denyallow
                .iter()
                .any(|exception| exception.zone_of(name))
    }
}

#[derive(Default)]
pub(super) struct Parsed {
    pub simple: HashSet<SimpleRule>,
    pub rules: Vec<Rule>,
    pub disabled: HashSet<Rule>,
    pub ignored: usize,
}

pub(super) fn parse(text: &str, allow: bool) -> io::Result<Parsed> {
    let mut parsed = Parsed::default();
    let mut unique = HashMap::new();
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
        let mut add = |rule: Rule, disabled: bool| {
            if disabled {
                parsed.disabled.insert(rule);
            } else {
                match rule.into_simple() {
                    Ok(rule) => {
                        parsed.simple.insert(rule);
                    }
                    Err(rule) => {
                        let order = unique.len();
                        unique.entry(rule).or_insert(order);
                    }
                }
            }
        };
        let classification = line
            .split_once('#')
            .filter(|(body, _)| body.ends_with(char::is_whitespace))
            .map_or(line, |(body, _)| body);
        if classification.contains(['|', '^', '$', '*']) || classification.starts_with("@@") {
            if let Some((rule, disabled)) = adblock(line, allow) {
                add(rule, disabled);
            } else {
                parsed.ignored += 1;
            }
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
                    if let Some(name) = domain(host) {
                        add(Rule::simple(name, action), false);
                    } else {
                        parsed.ignored += 1;
                    }
                }
            } else if fields.next().is_none() {
                if let Some(name) = domain(first) {
                    add(Rule::simple(name, action), false);
                } else {
                    parsed.ignored += 1;
                }
            } else {
                parsed.ignored += 1;
            }
        }
        if unique.len() + parsed.simple.len() + parsed.disabled.len() > MAX_RULES {
            return Err(io::Error::other(
                "trop de règles dans une liste de filtrage",
            ));
        }
    }
    let mut ordered: Vec<_> = unique.into_iter().collect();
    ordered.sort_unstable_by_key(|(_, order)| *order);
    parsed.rules = ordered.into_iter().map(|(rule, _)| rule).collect();
    Ok(parsed)
}

fn adblock(line: &str, user_allow: bool) -> Option<(Rule, bool)> {
    let (exception, body) = line
        .strip_prefix("@@")
        .map_or((false, line), |body| (true, body));
    let (pattern, options) = body.split_once('$').unwrap_or((body, ""));
    let selector = if pattern.is_empty() && !options.is_empty() {
        Selector::Mask("*".into())
    } else {
        selector(pattern)?
    };
    let mut conditions = Conditions::default();
    let mut important = false;
    let mut disabled = false;
    let mut seen = HashSet::new();
    // Les virgules échappées dans TXT ne séparent pas les modificateurs.
    let mut begin = 0;
    let mut escaped = false;
    let mut parts = Vec::new();
    for (index, byte) in options.bytes().enumerate() {
        if byte == b',' && !escaped {
            parts.push(&options[begin..index]);
            begin = index + 1;
        }
        escaped = byte == b'\\' && !escaped;
    }
    parts.push(&options[begin..]);
    for option in parts.into_iter().filter(|part| !part.is_empty()) {
        let (key, value) = option
            .split_once('=')
            .map_or((option, None), |(key, value)| (key, Some(value)));
        if !seen.insert(key) {
            return None;
        }
        match (key, value) {
            ("important", None) => important = true,
            ("badfilter", None) => disabled = true,
            ("dnstype", Some(value)) => {
                let mut includes = Vec::new();
                let mut excludes = Vec::new();
                for part in value.split('|') {
                    let (exclude, value) = part
                        .strip_prefix('~')
                        .map_or((false, part), |value| (true, value));
                    let kind = RecordType::from_str(&value.to_ascii_uppercase()).ok()?;
                    if matches!(kind, RecordType::Unknown(_)) {
                        return None;
                    }
                    if exclude {
                        excludes.push(kind);
                    } else {
                        includes.push(kind);
                    }
                }
                conditions.exclude_types = includes.is_empty();
                conditions.types = if includes.is_empty() {
                    excludes
                } else {
                    includes
                };
                conditions.types.sort();
                conditions.types.dedup();
                if conditions.types.is_empty() {
                    return None;
                }
            }
            ("denyallow", Some(value)) => {
                conditions.denyallow = value.split('|').map(dns_name).collect::<Option<_>>()?;
                conditions.denyallow.sort();
                conditions.denyallow.dedup();
            }
            ("dnsrewrite", Some(value)) => conditions.rewrite = Some(rewrite::parse(value)?),
            ("dnsrewrite", None) if exception => conditions.rewrite = Some(Rewrite::All),
            _ => return None,
        }
    }
    let action = if user_allow {
        Action::UserAllow
    } else {
        match (exception, important) {
            (false, false) => Action::Block,
            (true, false) => Action::Allow,
            (false, true) => Action::ImportantBlock,
            (true, true) => Action::ImportantAllow,
        }
    };
    let conditions = (!conditions.types.is_empty()
        || !conditions.denyallow.is_empty()
        || conditions.rewrite.is_some())
    .then(|| Box::new(conditions));
    Some((
        Rule {
            selector,
            action,
            conditions,
        },
        disabled,
    ))
}

fn selector(value: &str) -> Option<Selector> {
    let value = value.to_ascii_lowercase();
    if let Some(domain) = value
        .strip_prefix("||")
        .and_then(|value| value.strip_suffix('^'))
        && !domain.contains('*')
    {
        return Some(Selector::Suffix(dns_name(domain)?));
    }
    if !value.is_ascii()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'*' | b'^' | b'|')
        })
    {
        return None;
    }
    let (subdomains, value) = value
        .strip_prefix("||")
        .map_or((false, value.as_str()), |value| (true, value));
    let (start, value) = value
        .strip_prefix('|')
        .map_or((subdomains, value), |value| (true, value));
    let (end, value) = value
        .strip_suffix(['^', '|'])
        .map_or((false, value), |value| (true, value));
    if value.contains(['^', '|']) {
        return None;
    }
    let value = value.trim_end_matches('.');
    if value.is_empty() {
        return None;
    }
    // Le préfixe spécial est développé en deux motifs par l'index (apex et sous-domaines).
    let mask = format!(
        "{}{}{}{}",
        if subdomains { "||" } else { "" },
        if start { "" } else { "*" },
        value,
        if end { "" } else { "*" }
    );
    Some(Selector::Mask(mask))
}

pub(super) fn dns_name(value: &str) -> Option<Name> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return None;
    }
    let mut name = Name::from_ascii(value).ok()?.to_lowercase();
    name.set_fqdn(true);
    (!name.is_root()).then_some(name)
}

fn domain(value: &str) -> Option<Name> {
    if !value.trim_end_matches('.').contains('.')
        || value.parse::<IpAddr>().is_ok()
        || value
            .trim_end_matches('.')
            .eq_ignore_ascii_case("localhost.localdomain")
    {
        return None;
    }
    dns_name(value)
}
