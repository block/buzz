use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    valid_text, valid_time, BriefRunState, BriefSection, Classification, ContractError,
    MAX_ARRAY_ITEMS,
};

/// Current bounded status for one run.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefRunStatus {
    classification: Classification,
    run_id: String,
    schedule_id: String,
    sequence: u64,
    state: BriefRunState,
    updated_at: String,
    degraded_sections: Vec<BriefSection>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawBriefRunStatus {
    classification: Classification,
    run_id: String,
    schedule_id: String,
    sequence: u64,
    state: BriefRunState,
    updated_at: String,
    degraded_sections: Vec<BriefSection>,
    error: Option<String>,
}

impl TryFrom<Value> for BriefRunStatus {
    type Error = ContractError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let raw: RawBriefRunStatus = serde_json::from_value(value).map_err(|_| ContractError)?;
        if !valid_text(&raw.run_id)
            || !valid_text(&raw.schedule_id)
            || !valid_time(&raw.updated_at)
            || raw.degraded_sections.len() > MAX_ARRAY_ITEMS
            || raw.degraded_sections.iter().collect::<BTreeSet<_>>().len()
                != raw.degraded_sections.len()
            || raw.error.as_deref().is_some_and(|error| !valid_text(error))
        {
            return Err(ContractError);
        }
        Ok(Self {
            classification: raw.classification,
            run_id: raw.run_id,
            schedule_id: raw.schedule_id,
            sequence: raw.sequence,
            state: raw.state,
            updated_at: raw.updated_at,
            degraded_sections: raw.degraded_sections,
            error: raw.error,
        })
    }
}

impl BriefRunStatus {
    /// Return the native-owned run identity.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Return the native monotonic lifecycle sequence for this run.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Return the closed lifecycle state.
    pub const fn state(&self) -> BriefRunState {
        self.state
    }

    /// Return the trusted status timestamp used for bounded ordering.
    pub fn updated_at(&self) -> &str {
        &self.updated_at
    }
}
