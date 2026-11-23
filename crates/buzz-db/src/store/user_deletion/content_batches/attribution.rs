use super::{invalid, ContentRowCounts, PlannedAction, Result};
use serde::Serialize;
use std::collections::{hash_map::Entry, HashMap};

/// Mutations attributed to the same identity or attribution group.
#[derive(Debug, Default, Serialize)]
pub struct ContentActions {
    /// Rows selected for deletion.
    pub delete: ContentRowCounts,
    /// Rows retained after erasing personal content.
    pub redact_content: ContentRowCounts,
}

impl ContentActions {
    fn add(&mut self, row: &AttributedRow) {
        let counts = match row.action {
            PlannedAction::Delete => &mut self.delete,
            PlannedAction::Redact => &mut self.redact_content,
        };
        counts.add(row.table, row.kind);
    }
}

/// One identity in the community's content deletion scope.
#[derive(Debug, Serialize)]
pub struct ContentTargetCounts {
    /// Hex public key selected by the planner.
    pub pubkey: String,
    /// `human` or `owned_agent` in this community.
    pub role: &'static str,
    /// Operations attributable exclusively to this identity.
    #[serde(flatten)]
    pub counts: ContentActions,
}

/// Disjoint attribution of the read-only content plan.
#[derive(Debug, Serialize)]
pub struct ContentAttribution {
    /// Human and individually owned agents, including identities with no work.
    pub targets: Vec<ContentTargetCounts>,
    /// Rows selected by evidence for multiple in-scope identities.
    pub shared: ContentActions,
    /// Rows without proof of an in-scope identity.
    pub unattributed: ContentActions,
}

pub(super) struct AttributionPlan {
    targets: HashMap<Vec<u8>, usize>,
    counts: ContentAttribution,
    rows: HashMap<String, AttributedRow>,
}

#[derive(Clone, Copy, Default)]
enum Subjects {
    #[default]
    None,
    One(usize),
    Multiple,
}

impl Subjects {
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::None, other) | (other, Self::None) => other,
            (Self::One(left), Self::One(right)) if left == right => self,
            _ => Self::Multiple,
        }
    }
}

struct AttributedRow {
    table: &'static str,
    action: PlannedAction,
    kind: Option<i64>,
    subjects: Subjects,
}

impl AttributedRow {
    fn merge(&mut self, other: Self) {
        self.subjects = self.subjects.merge(other.subjects);
    }
}

impl AttributionPlan {
    pub(super) fn new(human: &str, keys: &[Vec<u8>]) -> Self {
        let targets = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, key)| (key, i))
            .collect();
        let counts = ContentAttribution {
            targets: keys
                .iter()
                .map(|key| {
                    let pubkey = hex::encode(key);
                    let role = if pubkey == human {
                        "human"
                    } else {
                        "owned_agent"
                    };
                    ContentTargetCounts {
                        pubkey,
                        role,
                        counts: ContentActions::default(),
                    }
                })
                .collect(),
            shared: ContentActions::default(),
            unattributed: ContentActions::default(),
        };
        Self {
            targets,
            counts,
            rows: HashMap::new(),
        }
    }

    pub(super) fn insert(
        &mut self,
        identity: String,
        table: &'static str,
        action: PlannedAction,
        kind: Option<i64>,
        evidence: &[Vec<u8>],
        max_items: usize,
    ) -> Result<bool> {
        let subjects = evidence
            .iter()
            .filter_map(|key| self.targets.get(key))
            .fold(Subjects::None, |subjects, &target| {
                subjects.merge(Subjects::One(target))
            });
        let row = AttributedRow {
            table,
            action,
            kind,
            subjects,
        };
        let at_limit = self.rows.len() >= max_items;
        match self.rows.entry(identity) {
            Entry::Occupied(mut previous) => {
                previous.get_mut().merge(row);
                Ok(false)
            }
            Entry::Vacant(entry) => {
                if at_limit {
                    return Err(invalid(
                        "preview item limit exceeded; output is incomplete; increase --max-items",
                    ));
                }
                entry.insert(row);
                Ok(true)
            }
        }
    }

    pub(super) fn finish(mut self) -> ContentAttribution {
        for row in self.rows.into_values() {
            let counts = match row.subjects {
                Subjects::None => &mut self.counts.unattributed,
                Subjects::One(index) => &mut self.counts.targets[index].counts,
                Subjects::Multiple => &mut self.counts.shared,
            };
            counts.add(&row);
        }
        self.counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_evidence_merges_without_consuming_another_row() {
        let human = vec![1; 32];
        let agent = vec![2; 32];
        let idle = vec![3; 32];
        let mut report =
            AttributionPlan::new(&hex::encode(&human), &[human.clone(), agent.clone(), idle]);
        for (evidence, new) in [
            (&[][..], true),
            (&[human.clone(), human][..], false),
            (&[agent][..], false),
        ] {
            assert_eq!(
                report
                    .insert(
                        "one".into(),
                        "push_wake_outbox",
                        PlannedAction::Delete,
                        None,
                        evidence,
                        2
                    )
                    .unwrap(),
                new
            );
        }
        assert!(report
            .insert(
                "two".into(),
                "events",
                PlannedAction::Redact,
                Some(9),
                &[vec![4; 32]],
                2
            )
            .unwrap());
        let counts = report.finish();
        assert_eq!(counts.targets.len(), 3);
        assert!(counts
            .targets
            .iter()
            .all(|target| target.counts.delete.rows == 0));
        assert_eq!(counts.shared.delete.rows, 1);
        assert_eq!(
            counts.unattributed.redact_content.events_by_kind.get(&9),
            Some(&1)
        );
    }
}
