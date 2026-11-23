//! Bounded counts accumulated from the selected rows in a community preview.
use super::{invalid, Operation, Options};
use crate::Result;
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Default, Serialize)]
pub(super) struct AccountPreview {
    delete: DeleteCounts,
    clear_attribution: AttributionCounts,
    targets: Vec<TargetCounts>,
}

#[derive(Debug, Serialize)]
struct TargetCounts {
    pubkey: String,
    role: &'static str,
    delete: RowDeletes,
    clear_attribution: FieldClears,
}

#[derive(Debug, Default, Serialize)]
struct DeleteCounts {
    #[serde(flatten)]
    counts: RowDeletes,
    by_subject: Subjects,
}

#[derive(Debug, Default, Serialize)]
struct AttributionCounts {
    #[serde(flatten)]
    counts: FieldClears,
    by_subject: Subjects,
}

#[derive(Debug, Default, Serialize)]
struct RowDeletes {
    rows: usize,
    by_table: BTreeMap<&'static str, usize>,
    events_by_kind: BTreeMap<i64, usize>,
}

#[derive(Debug, Default, Serialize)]
struct FieldClears {
    fields: usize,
    by_table: BTreeMap<&'static str, BTreeMap<&'static str, usize>>,
}

#[derive(Debug, Default, Serialize)]
struct Subjects {
    user: usize,
    owned_agents: usize,
}

impl AccountPreview {
    pub(super) fn new(human: &str, keys: &[String]) -> Self {
        let mut targets: Vec<_> = keys
            .iter()
            .map(|key| TargetCounts {
                pubkey: key.clone(),
                role: if key == human { "human" } else { "owned_agent" },
                delete: RowDeletes::default(),
                clear_attribution: FieldClears::default(),
            })
            .collect();
        targets.sort_unstable_by(|left, right| left.pubkey.cmp(&right.pubkey));
        Self {
            targets,
            ..Self::default()
        }
    }

    pub(super) fn record(
        &mut self,
        options: &Options,
        op: &Operation,
        keys: &[Value],
    ) -> Result<()> {
        let count = keys.len();
        let index = self
            .targets
            .binary_search_by(|target| target.pubkey.cmp(&options.pubkey))
            .map_err(|_| invalid("missing account preview target"))?;
        let target = &mut self.targets[index];
        let subjects = if let Some(field) = op.action.strip_prefix("clear_") {
            // The cleared field is attributable to the selected subject. The
            // row itself is shared or belongs to another identity.
            for counts in [
                &mut self.clear_attribution.counts,
                &mut target.clear_attribution,
            ] {
                counts.fields += count;
                let fields = counts.by_table.entry(op.table).or_default();
                *fields.entry(field).or_default() += count;
            }
            &mut self.clear_attribution.by_subject
        } else {
            for counts in [&mut self.delete.counts, &mut target.delete] {
                counts.rows += count;
                *counts.by_table.entry(op.table).or_default() += count;
                if op.table == "events" {
                    for key in keys {
                        let kind = key["kind"]
                            .as_i64()
                            .ok_or_else(|| invalid("missing account event kind"))?;
                        *counts.events_by_kind.entry(kind).or_default() += 1;
                    }
                }
            }
            &mut self.delete.by_subject
        };
        if options.pubkey == options.target {
            subjects.user += count;
        } else {
            subjects.owned_agents += count;
        }
        Ok(())
    }
}
