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
}

#[derive(Debug, Default, Serialize)]
struct DeleteCounts {
    rows: usize,
    by_table: BTreeMap<&'static str, usize>,
    events_by_kind: BTreeMap<i64, usize>,
    by_subject: Subjects,
}

#[derive(Debug, Default, Serialize)]
struct AttributionCounts {
    fields: usize,
    by_table: BTreeMap<&'static str, BTreeMap<&'static str, usize>>,
    by_subject: Subjects,
}

#[derive(Debug, Default, Serialize)]
struct Subjects {
    user: usize,
    owned_agents: usize,
}

impl AccountPreview {
    pub(super) fn record(
        &mut self,
        options: &Options,
        op: &Operation,
        keys: &[Value],
    ) -> Result<()> {
        let count = keys.len();
        let subjects = if let Some(field) = op.action.strip_prefix("clear_") {
            self.clear_attribution.fields += count;
            let fields = self.clear_attribution.by_table.entry(op.table).or_default();
            *fields.entry(field).or_default() += count;
            &mut self.clear_attribution.by_subject
        } else {
            self.delete.rows += count;
            *self.delete.by_table.entry(op.table).or_default() += count;
            if op.table == "events" {
                for key in keys {
                    let kind = key["kind"]
                        .as_i64()
                        .ok_or_else(|| invalid("missing account event kind"))?;
                    *self.delete.events_by_kind.entry(kind).or_default() += 1;
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
