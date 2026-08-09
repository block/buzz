use std::collections::BTreeSet;

use super::{BriefSection, MAX_ARRAY_ITEMS, RAG_MEMORY_SECTIONS};

pub(super) fn degrade_all(
    degraded: &mut BTreeSet<BriefSection>,
    limitations: &mut BTreeSet<String>,
    limitation: &str,
) {
    degraded.extend(RAG_MEMORY_SECTIONS);
    limitations.insert(limitation.to_string());
}

pub(super) fn bounded_limitations(limitations: BTreeSet<String>) -> Vec<String> {
    if limitations.len() <= MAX_ARRAY_ITEMS {
        return limitations.into_iter().collect();
    }
    let omitted = limitations.len() - (MAX_ARRAY_ITEMS - 1);
    let mut bounded = limitations
        .into_iter()
        .take(MAX_ARRAY_ITEMS - 1)
        .collect::<Vec<_>>();
    bounded.push(format!(
        "{omitted} additional source limitations omitted after the canonical limit."
    ));
    bounded
}
