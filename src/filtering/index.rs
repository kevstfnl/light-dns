use super::parser::{self, Action, MAX_RULES};
use hickory_proto::rr::Name;
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[derive(Default)]
pub(crate) struct FilterIndex {
    rules: HashMap<Name, Action>,
}

impl FilterIndex {
    pub(super) fn compile(texts: &[(String, bool)], allowlist: &[Name]) -> io::Result<Self> {
        let mut rules = HashSet::new();
        let mut disabled = HashSet::new();
        let mut ignored = 0;
        for (text, allow) in texts {
            let parsed = parser::parse(text, *allow)?;
            if parsed.rules.is_empty() && parsed.disabled.is_empty() && parsed.ignored != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "liste sans règle DNS compatible",
                ));
            }
            ignored += parsed.ignored;
            rules.extend(parsed.rules);
            disabled.extend(parsed.disabled);
            if rules.len() + disabled.len() + allowlist.len() > MAX_RULES {
                return Err(io::Error::other("limite globale de 500000 règles dépassée"));
            }
        }
        let mut index = HashMap::with_capacity(rules.len());
        for rule in rules.difference(&disabled) {
            index
                .entry(rule.name.clone())
                .and_modify(|action: &mut Action| *action = (*action).max(rule.action))
                .or_insert(rule.action);
        }
        for name in allowlist {
            index.insert(name.clone(), Action::UserAllow);
        }
        if ignored != 0 {
            tracing::warn!(
                ignored,
                "règles incompatibles avec le filtrage DNS ignorées"
            );
        }
        Ok(Self { rules: index })
    }

    pub(crate) fn is_blocked(&self, name: &Name) -> bool {
        if self.rules.is_empty() {
            return false;
        }
        let mut candidate = name.clone();
        let mut strongest = None;
        while !candidate.is_root() {
            if let Some(action) = self.rules.get(&candidate) {
                strongest = strongest.max(Some(*action));
                if strongest == Some(Action::UserAllow) {
                    return false;
                }
            }
            candidate = candidate.base_name();
        }
        matches!(strongest, Some(Action::Block | Action::ImportantBlock))
    }

    pub(super) fn len(&self) -> usize {
        self.rules.len()
    }
}
