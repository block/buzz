use serde::{Deserialize, Serialize};

use super::RespondTo;

/// A relay-published agent directory entry (kind:10100), as handed to the
/// frontend by `list_relay_agents`.
///
/// This type crosses two boundaries with opposite casing conventions, and
/// getting either wrong fails silently — the frontend just sees `undefined`,
/// and every relay-published agent quietly stops being mentionable.
///
/// * **Serializes camelCase.** The TypeScript `RelayAgent` type reads
///   `agentType` / `channelIds` / `respondTo` / `respondToAllowlist`. Emitting
///   snake_case here left all four `undefined`, so `relayAgentIsSharedWithUser`
///   could never return true for any relay agent.
/// * **Deserializes either casing.** kind:10100 event content is snake_case
///   (see `agents_from_events`), so every renamed field keeps a snake_case
///   `alias`. Dropping those would break directory parsing.
///
/// Both directions are pinned by tests in `types/tests.rs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelayAgentInfo {
    pub pubkey: String,
    pub name: String,
    #[serde(alias = "agent_type")]
    pub agent_type: String,
    pub channels: Vec<String>,
    #[serde(default, alias = "channel_ids")]
    pub channel_ids: Vec<String>,
    pub capabilities: Vec<String>,
    pub status: String,
    #[serde(default, alias = "respond_to")]
    pub respond_to: Option<RespondTo>,
    #[serde(default, alias = "respond_to_allowlist")]
    pub respond_to_allowlist: Vec<String>,
}
