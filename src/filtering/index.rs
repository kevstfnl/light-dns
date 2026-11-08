use super::{
    parser::{self, Action, MAX_RULES, Rule, Selector},
    rewrite::Rewrite,
};
use globset::Glob;
use hickory_proto::{
    op::{Message, ResponseCode},
    rr::{Name, RData, RecordType},
};
use regex_automata::{Input, MatchKind, PatternSet, meta::Regex};
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[derive(Default)]
pub(crate) struct FilterIndex {
    rules: HashMap<Name, Action>,
    advanced: Vec<Rule>,
    suffixes: HashMap<Name, Vec<usize>>,
    masks: Option<Regex>,
    mask_rules: Vec<usize>,
    count: usize,
}

pub(super) struct FilterList {
    pub text: String,
    pub allow: bool,
    pub allow_dnsrewrite: bool,
}

pub(crate) enum Decision {
    Pass,
    Allow,
    Block,
    Rewrite {
        code: ResponseCode,
        data: Vec<RData>,
    },
}

impl FilterIndex {
    pub(super) fn compile(texts: &[FilterList], allowlist: &[Name]) -> io::Result<Self> {
        let mut rules = HashMap::new();
        let mut simple = HashSet::new();
        let mut disabled_simple = HashSet::new();
        let mut disabled = HashSet::new();
        let mut ignored = 0;
        let mut untrusted_rewrites = 0;
        for list in texts {
            let parsed = parser::parse(&list.text, list.allow, list.allow_dnsrewrite)?;
            if parsed.rules.is_empty()
                && parsed.simple.is_empty()
                && parsed.disabled.is_empty()
                && parsed.untrusted_rewrites == 0
                && parsed.ignored != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "liste sans règle DNS compatible",
                ));
            }
            ignored += parsed.ignored;
            untrusted_rewrites += parsed.untrusted_rewrites;
            for rule in parsed.rules {
                let order = rules.len();
                rules.entry(rule).or_insert(order);
            }
            simple.extend(parsed.simple);
            for rule in parsed.disabled {
                match rule.into_simple() {
                    Ok(rule) => {
                        disabled_simple.insert(rule);
                    }
                    Err(rule) => {
                        disabled.insert(rule);
                    }
                }
            }
            if rules.len() + simple.len() + disabled.len() + disabled_simple.len() + allowlist.len()
                > MAX_RULES
            {
                return Err(io::Error::other("limite globale de 500000 règles dépassée"));
            }
        }
        // Préallouer comme l'ancien index : les tables intermédiaires de croissance
        // restaient sinon retenues par l'allocateur après le chargement des listes.
        let mut index = Self {
            rules: HashMap::with_capacity(simple.len() + allowlist.len()),
            ..Self::default()
        };
        for rule in simple
            .into_iter()
            .filter(|rule| !disabled_simple.contains(rule))
        {
            index.count += 1;
            index
                .rules
                .entry(rule.name)
                .and_modify(|action| *action = (*action).max(rule.action))
                .or_insert(rule.action);
        }
        let mut expressions = Vec::new();
        let mut ordered: Vec<_> = rules.into_iter().collect();
        ordered.sort_unstable_by_key(|(_, order)| *order);
        for (rule, _) in ordered
            .into_iter()
            .filter(|(rule, _)| !disabled.contains(rule))
        {
            index.count += 1;
            if let Selector::Suffix(name) = &rule.selector {
                index
                    .suffixes
                    .entry(name.clone())
                    .or_default()
                    .push(index.advanced.len());
            } else if let Selector::Mask(mask) = &rule.selector {
                let patterns = if let Some(mask) = mask.strip_prefix("||") {
                    vec![mask.to_owned(), format!("*.{mask}")]
                } else {
                    vec![mask.clone()]
                };
                for pattern in patterns {
                    expressions.push(
                        Glob::new(&pattern)
                            .map_err(io::Error::other)?
                            .regex()
                            .to_owned(),
                    );
                    index.mask_rules.push(index.advanced.len());
                }
            }
            index.advanced.push(rule);
        }
        if !expressions.is_empty() {
            // GlobSet classe les noms DNS comme chemins avec une extension commune et
            // peut tester chaque regex séparément. Compiler explicitement un automate
            // multi-motifs évite ce coût linéaire dans les listes DNS.
            index.masks = Some(
                Regex::builder()
                    .syntax(regex_automata::util::syntax::Config::new().utf8(false))
                    .configure(
                        Regex::config()
                            .match_kind(MatchKind::All)
                            .nfa_size_limit(Some(10 * 1024 * 1024)),
                    )
                    .build_many(&expressions)
                    .map_err(io::Error::other)?,
            );
        }
        for name in allowlist {
            let mut name = name.to_lowercase();
            name.set_fqdn(true);
            index.rules.insert(name, Action::UserAllow);
            index.count += 1;
        }
        if ignored != 0 {
            tracing::warn!(
                ignored,
                "règles incompatibles avec le filtrage DNS ignorées"
            );
        }
        if untrusted_rewrites != 0 {
            tracing::warn!(
                untrusted_rewrites,
                "règles dnsrewrite distantes ignorées : source non autorisée"
            );
        }
        Ok(index)
    }

    pub(crate) fn decide(&self, name: &Name, kind: RecordType) -> Decision {
        if self.count == 0 {
            return Decision::Pass;
        }
        let mut candidate = name.clone();
        let mut strongest = None;
        let mut matched = Vec::new();
        while !candidate.is_root() {
            if let Some(action) = self.rules.get(&candidate) {
                strongest = strongest.max(Some(*action));
                if strongest == Some(Action::UserAllow) {
                    return Decision::Allow;
                }
            }
            if !self.suffixes.is_empty()
                && let Some(rules) = self.suffixes.get(&candidate)
            {
                matched.extend_from_slice(rules);
            }
            candidate = candidate.base_name();
        }
        if let Some(masks) = &self.masks {
            let text = name.to_ascii().to_ascii_lowercase();
            let mut hits = PatternSet::new(masks.pattern_len());
            masks.which_overlapping_matches(
                &Input::new(text.trim_end_matches('.').as_bytes()),
                &mut hits,
            );
            matched.extend(hits.iter().map(|id| self.mask_rules[id.as_usize()]));
        }
        if matched.is_empty() {
            return decision(strongest);
        }
        matched.sort_unstable();
        matched.dedup();
        let matched: Vec<_> = matched
            .into_iter()
            .map(|id| &self.advanced[id])
            .filter(|rule| rule.applies(name, kind))
            .collect();
        for rule in &matched {
            if rule.rewrite().is_none() {
                strongest = strongest.max(Some(rule.action));
            }
        }
        if strongest == Some(Action::UserAllow) {
            return Decision::Allow;
        }
        let mut replacements = Vec::new();
        for rule in &matched {
            let Some(rewrite) = rule.rewrite() else {
                continue;
            };
            if matches!(
                rule.action,
                Action::Allow | Action::ImportantAllow | Action::UserAllow
            ) {
                continue;
            }
            let excepted = matched.iter().any(|exception| {
                matches!(
                    exception.action,
                    Action::Allow | Action::ImportantAllow | Action::UserAllow
                ) && exception
                    .rewrite()
                    .is_some_and(|value| *value == Rewrite::All || value == rewrite)
            });
            if !excepted && !replacements.contains(&rewrite) {
                replacements.push(rewrite);
            }
        }
        // RCODE, puis CNAME, puis union des RR ; l'ordre source/ligne départage les conflits.
        if let Some(Rewrite::Value { code, data }) = replacements
            .iter()
            .find(|rewrite| matches!(rewrite, Rewrite::Value { data: None, .. }))
        {
            return Decision::Rewrite {
                code: *code,
                data: data.iter().cloned().collect(),
            };
        }
        if let Some(Rewrite::Value { code, data }) = replacements.iter().find(|rewrite| {
            matches!(
                rewrite,
                Rewrite::Value {
                    data: Some(RData::CNAME(_)),
                    ..
                }
            )
        }) {
            return Decision::Rewrite {
                code: *code,
                data: data.iter().cloned().collect(),
            };
        }
        if !replacements.is_empty() {
            let data = replacements
                .iter()
                .filter_map(|rewrite| match rewrite {
                    Rewrite::Value { data, .. } => data.as_ref(),
                    _ => None,
                })
                .cloned()
                .collect();
            return Decision::Rewrite {
                code: ResponseCode::NoError,
                data,
            };
        }
        decision(strongest)
    }

    /// Ne parcourt que la chaîne liée à la question, sans requête DNS supplémentaire.
    pub(crate) fn blocks_cname(
        &self,
        name: &Name,
        response: &Message,
        allowed: bool,
    ) -> Result<bool, ()> {
        if self.count == 0
            || !response
                .answers
                .iter()
                .any(|rr| matches!(rr.data, RData::CNAME(_)))
        {
            return Ok(false);
        }
        let mut current = name;
        let mut seen = Vec::new();
        loop {
            let mut target = None;
            for record in &response.answers {
                if record.name == *current
                    && record.dns_class == hickory_proto::rr::DNSClass::IN
                    && let RData::CNAME(cname) = &record.data
                {
                    if target.is_some_and(|previous| previous != &cname.0) {
                        return Err(());
                    }
                    target = Some(&cname.0);
                }
            }
            let Some(target) = target else {
                return Ok(false);
            };
            if seen.len() == 16 || target == name || seen.contains(&target) {
                return Err(());
            }
            if !allowed && matches!(self.decide(target, RecordType::CNAME), Decision::Block) {
                return Ok(true);
            }
            seen.push(target);
            current = target;
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }
}

fn decision(action: Option<Action>) -> Decision {
    match action {
        Some(Action::Block | Action::ImportantBlock) => Decision::Block,
        Some(Action::Allow | Action::ImportantAllow | Action::UserAllow) => Decision::Allow,
        None => Decision::Pass,
    }
}
