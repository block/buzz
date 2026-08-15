use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{unique_valid_text_array, valid_text, CitedFinding, Classification, ContractError};

/// A proposal which remains pending in the immutable brief until the CO directs it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingProposal {
    classification: Classification,
    action_id: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    alternative_text: Option<String>,
    approval_state: PendingApprovalState,
    source_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PendingApprovalState {
    Pending,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RawPendingProposal {
    classification: Classification,
    action_id: String,
    text: String,
    #[serde(default)]
    alternative_text: Option<String>,
    approval_state: PendingApprovalState,
    #[serde(default)]
    source_ids: Option<Vec<String>>,
}

impl PendingProposal {
    /// Returns the pending action text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the credible alternative course of action when one was supplied.
    pub fn alternative_text(&self) -> Option<&str> {
        self.alternative_text.as_deref()
    }

    /// Returns the evidence admitted for this pending action.
    pub fn source_ids(&self) -> &[String] {
        &self.source_ids
    }

    pub(crate) fn as_finding(&self) -> CitedFinding {
        CitedFinding {
            classification: self.classification,
            text: self.text.clone(),
            source_ids: self.source_ids.clone(),
        }
    }
}

pub(super) fn parse_raw_proposals(
    proposals: Vec<RawPendingProposal>,
    findings: &[CitedFinding],
    ledger_ids: &BTreeSet<String>,
    require_explicit_sources: bool,
) -> Result<Vec<PendingProposal>, ContractError> {
    let fallback_source_ids = findings
        .iter()
        .flat_map(|finding| finding.source_ids().iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    proposals
        .into_iter()
        .map(|proposal| {
            if !valid_text(&proposal.action_id)
                || !valid_text(&proposal.text)
                || proposal
                    .alternative_text
                    .as_deref()
                    .is_some_and(|alternative| !valid_text(alternative))
            {
                return Err(ContractError);
            }
            let mut source_ids = match proposal.source_ids {
                Some(source_ids) => source_ids,
                None if !require_explicit_sources && !fallback_source_ids.is_empty() => {
                    fallback_source_ids.clone()
                }
                None => return Err(ContractError),
            };
            if source_ids.is_empty()
                || !unique_valid_text_array(&source_ids)
                || source_ids
                    .iter()
                    .any(|source_id| !ledger_ids.contains(source_id))
            {
                return Err(ContractError);
            }
            source_ids.sort();
            Ok(PendingProposal {
                classification: proposal.classification,
                action_id: proposal.action_id,
                text: proposal.text,
                alternative_text: proposal.alternative_text,
                approval_state: proposal.approval_state,
                source_ids,
            })
        })
        .collect()
}
