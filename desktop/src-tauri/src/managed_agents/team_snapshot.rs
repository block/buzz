//! `buzz-team-snapshot v1` — manifest type, encoder, and decoder.
//!
//! A team snapshot is a portable, shareable representation of a team: its
//! header (name, description) plus a `members` array where each member
//! reuses the existing `AgentSnapshot` type from `agent_snapshot.rs`.
//!
//! Two encodings:
//!   - `.team.json` — canonical; memory inclusion is a P4.2 product decision.
//!   - `.team.png` — 1×1 placeholder PNG with manifest in a `buzz_team_snapshot`
//!     tEXt chunk; **all members MUST have `memory.level == None` and empty
//!     `memory.entries`**. PNG files are casually shared — plaintext memory must
//!     never appear in them. Since `TeamRecord` has no team-level avatar, the
//!     image body is always the 1×1 placeholder; member avatars are carried in
//!     each `AgentSnapshot.profile.avatar_data_url`.
//!
//! **Old `.team.json` files** (flat `{version:1, type:"team", …}` schema) carry
//! no `format` discriminator. The caller's legacy-reject path handles them; this
//! decoder never sees them.
//!
//! **ZIP is NOT in v1** — consistent with the agent-snapshot policy.
//!
//! # Secret exclusion
//!
//! Per-member exclusions are inherited from `AgentSnapshot` — secrets are
//! excluded by construction in `agent_snapshot::build_snapshot`. No
//! team-level secrets are introduced by this wrapper.
//!
//! # Memory-consistency invariant
//!
//! Any member with `memory.level == None` and non-empty `memory.entries` is
//! malformed. `validate_member_memory_consistency` enforces this on both the
//! JSON decode path (via `validate_team_snapshot`) and the PNG encode path (via
//! `encode_team_snapshot_png`) so the rule is single-sourced.

// Items are `pub(crate)` for P4.2 callers; suppress dead-code lint until then.
#![allow(dead_code)]

use base64::{engine::general_purpose::STANDARD, Engine as _};
use png::Decoder;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

use crate::managed_agents::{
    agent_snapshot::{make_png_with_text, validate_snapshot, AgentSnapshot, MemoryLevel},
    TeamRecord,
};

// ── Constants ─────────────────────────────────────────────────────────────────

/// tEXt chunk keyword used in `.team.png` files.
pub const PNG_CHUNK_KEYWORD: &str = "buzz_team_snapshot";

/// Format discriminator — used for sniffing and validation.
pub const FORMAT_DISCRIMINATOR: &str = "buzz-team-snapshot";

/// Version of the manifest format produced by this module.
pub const FORMAT_VERSION: u32 = 1;

// ── Manifest sub-types ────────────────────────────────────────────────────────

/// Team-level metadata carried in the snapshot header.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshotMeta {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

// ── Top-level manifest ────────────────────────────────────────────────────────

/// The top-level `buzz-team-snapshot v1` manifest.
///
/// Serializes to / from JSON. Embedded in `.team.json` directly, or in the
/// `buzz_team_snapshot` tEXt chunk of a `.team.png` (base64-encoded).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TeamSnapshot {
    /// Fixed discriminator for format sniffing.
    pub format: String,
    /// Schema version. This module produces version 1.
    pub version: u32,
    /// Team-level metadata.
    pub team: TeamSnapshotMeta,
    /// One `AgentSnapshot` per team member. Order is preserved.
    pub members: Vec<AgentSnapshot>,
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Construct a `TeamSnapshot` from a `TeamRecord` and pre-built member snapshots.
///
/// Members are pre-built by the caller via `agent_snapshot::build_snapshot` —
/// this function does not call into Tauri or perform I/O. Same purity contract
/// as `build_snapshot`: pure assembly, deterministic, testable without AppHandle.
pub fn build_team_snapshot(team: &TeamRecord, members: Vec<AgentSnapshot>) -> TeamSnapshot {
    TeamSnapshot {
        format: FORMAT_DISCRIMINATOR.to_string(),
        version: FORMAT_VERSION,
        team: TeamSnapshotMeta {
            name: team.name.clone(),
            description: team.description.clone(),
        },
        members,
    }
}

// ── JSON encoding / decoding ──────────────────────────────────────────────────

/// Encode the manifest to pretty-printed JSON bytes.
pub fn encode_team_snapshot_json(snapshot: &TeamSnapshot) -> Result<Vec<u8>, String> {
    serde_json::to_vec_pretty(snapshot)
        .map_err(|e| format!("Failed to serialize team snapshot: {e}"))
}

/// Decode a manifest from JSON bytes.
pub fn decode_team_snapshot_json(bytes: &[u8]) -> Result<TeamSnapshot, String> {
    let snapshot: TeamSnapshot =
        serde_json::from_slice(bytes).map_err(|e| format!("Invalid team snapshot JSON: {e}"))?;
    validate_team_snapshot(&snapshot)?;
    Ok(snapshot)
}

// ── PNG encoding / decoding ───────────────────────────────────────────────────

/// Encode a team snapshot into a `.team.png`.
///
/// The image body is a 1×1 transparent placeholder — `TeamRecord` has no
/// team-level avatar; each member's avatar lives in their own
/// `AgentSnapshot.profile.avatar_data_url`. The manifest is embedded in the
/// `buzz_team_snapshot` tEXt chunk (base64-encoded JSON).
///
/// **Rejects** any snapshot where ANY member carries memory — either via a
/// non-`None` `memory.level` or a non-empty `memory.entries`. PNG images are
/// casually shared and would expose plaintext memory.
pub fn encode_team_snapshot_png(snapshot: &TeamSnapshot) -> Result<Vec<u8>, String> {
    validate_team_png_has_no_member_memory(snapshot)?;

    let json_bytes = encode_team_snapshot_json(snapshot)?;
    let chunk_text = STANDARD.encode(&json_bytes);

    // No team-level avatar: always use the 1×1 placeholder.
    // Each member's avatar is carried within their own AgentSnapshot.
    make_png_with_text(PNG_CHUNK_KEYWORD, &chunk_text)
}

/// Decode a manifest from a `.team.png` tEXt chunk.
pub fn decode_team_snapshot_png(png_bytes: &[u8]) -> Result<TeamSnapshot, String> {
    let decoder = Decoder::new(Cursor::new(png_bytes));
    let reader = decoder
        .read_info()
        .map_err(|e| format!("Invalid PNG: {e}"))?;
    let info = reader.info();

    let chunk_text = info
        .uncompressed_latin1_text
        .iter()
        .find(|c| c.keyword == PNG_CHUNK_KEYWORD)
        .map(|c| c.text.as_str())
        .ok_or_else(|| "PNG does not contain a buzz_team_snapshot tEXt chunk".to_string())?;

    let json_bytes = STANDARD
        .decode(chunk_text.trim())
        .map_err(|e| format!("Invalid base64 in PNG chunk: {e}"))?;

    let snapshot = decode_team_snapshot_json(&json_bytes)?;
    validate_team_png_has_no_member_memory(&snapshot)?;
    Ok(snapshot)
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Enforce the PNG no-member-memory invariant on both the encode and decode paths.
///
/// PNG files are casually shared — any member carrying memory (level != `None`
/// or non-empty entries) would expose plaintext memory. This is a hard format
/// invariant: our encoder never produces such a PNG, so any that passes this
/// check on decode is malformed or malicious.
fn validate_team_png_has_no_member_memory(snapshot: &TeamSnapshot) -> Result<(), String> {
    for (i, member) in snapshot.members.iter().enumerate() {
        if member.memory.level != MemoryLevel::None || !member.memory.entries.is_empty() {
            return Err(format!(
                "Cannot write memory to a .team.png file — member {i} ({:?}) has memory. \
                 PNG images are casually shared and would expose memory as plaintext.",
                member.definition.name
            ));
        }
    }
    Ok(())
}

/// Assert that a member's memory section is internally consistent.
///
/// Rejects `memory.level == None` with non-empty `memory.entries` — this is a
/// malformed state that the builder can never produce but a crafted payload
/// could. Single-sourced so the same rule applies on both the PNG encode path
/// and the JSON decode path.
fn validate_member_memory_consistency(idx: usize, member: &AgentSnapshot) -> Result<(), String> {
    if member.memory.level == MemoryLevel::None && !member.memory.entries.is_empty() {
        return Err(format!(
            "member {idx} ({:?}) has memory.level 'none' but non-empty entries — \
             this is a malformed snapshot",
            member.definition.name
        ));
    }
    Ok(())
}

/// Validate that the manifest has the correct format/version and required
/// fields. Returns an error string on failure.
///
/// Calls `agent_snapshot::validate_snapshot` for each member so the per-member
/// contract (non-empty name, correct format/version) is enforced at decode time.
/// Also checks the memory-consistency invariant per member via
/// `validate_member_memory_consistency`.
pub(crate) fn validate_team_snapshot(snapshot: &TeamSnapshot) -> Result<(), String> {
    if snapshot.format != FORMAT_DISCRIMINATOR {
        return Err(format!(
            "Unsupported team snapshot format: {:?} (expected {:?})",
            snapshot.format, FORMAT_DISCRIMINATOR
        ));
    }
    if snapshot.version != 1 {
        return Err(format!(
            "Unsupported team snapshot version: {} (expected 1)",
            snapshot.version
        ));
    }
    if snapshot.team.name.trim().is_empty() {
        return Err("Team snapshot team.name is empty".to_string());
    }
    if snapshot.members.is_empty() {
        return Err("Team snapshot must have at least one member".to_string());
    }
    for (i, member) in snapshot.members.iter().enumerate() {
        validate_snapshot(member).map_err(|e| format!("Team member {i} is invalid: {e}"))?;
        validate_member_memory_consistency(i, member)
            .map_err(|e| format!("Team member {i} memory is malformed: {e}"))?;
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::{
        agent_snapshot::{build_snapshot, AgentSnapshotMemory, AgentSnapshotMemoryEntry},
        types::{BackendKind, ManagedAgentRecord, RespondTo},
    };
    use std::collections::BTreeMap;

    /// Build a minimal `TeamRecord` for testing.
    fn team_record(name: &str) -> TeamRecord {
        TeamRecord {
            id: format!("{name}-id"),
            name: name.to_string(),
            description: Some(format!("{name} description")),
            persona_ids: vec![],
            is_builtin: false,
            source_dir: None,
            is_symlink: false,
            symlink_target: None,
            version: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-02T00:00:00Z".to_string(),
        }
    }

    /// Build a minimal `ManagedAgentRecord` for use as a team member.
    fn agent_record(name: &str) -> ManagedAgentRecord {
        ManagedAgentRecord {
            pubkey: format!("{name}-pubkey"),
            name: name.to_string(),
            display_name: Some(format!("{name} Display")),
            persona_id: Some("SENTINEL_PERSONA_ID".to_string()), // MUST NOT appear
            private_key_nsec: "nsec1secret".to_string(),         // MUST NOT appear
            auth_tag: Some("auth-tag-secret".to_string()),       // MUST NOT appear
            relay_url: "wss://relay.example.com".to_string(),    // MUST NOT appear
            avatar_url: Some(format!("https://example.com/{name}.png")),
            acp_command: "/usr/local/bin/acp".to_string(), // MUST NOT appear
            agent_command: "goose".to_string(),            // MUST NOT appear
            agent_command_override: None,
            agent_args: vec![],
            mcp_command: String::new(),
            turn_timeout_seconds: 120,
            idle_timeout_seconds: Some(30),
            max_turn_duration_seconds: Some(600),
            parallelism: 1,
            system_prompt: Some(format!("You are {name}.")),
            model: Some("claude-opus-4".to_string()),
            provider: Some("anthropic".to_string()),
            persona_source_version: None,
            env_vars: {
                let mut m = BTreeMap::new();
                m.insert("API_KEY".to_string(), "secret123".to_string()); // MUST NOT appear
                m
            },
            start_on_app_launch: false,
            auto_restart_on_config_change: true,
            runtime_pid: None,
            backend: BackendKind::Local,
            backend_agent_id: None,
            provider_binary_path: None,
            persona_team_dir: None,
            persona_name_in_team: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            last_started_at: None,
            last_stopped_at: None,
            last_exit_code: None,
            last_error: None,
            last_error_code: None,
            respond_to: RespondTo::default(),
            respond_to_allowlist: vec![],
            slug: Some(name.to_string()),
            runtime: Some("goose".to_string()),
            name_pool: vec![],
            is_builtin: false,
            is_active: true,
            source_team: Some("SENTINEL_SOURCE_TEAM".to_string()), // MUST NOT appear
            source_team_persona_slug: Some("SENTINEL_SLUG".to_string()), // MUST NOT appear
            definition_respond_to: None,
            definition_respond_to_allowlist: vec![],
            definition_parallelism: None,
            relay_mesh: None,
        }
    }

    /// Build a canonical two-member team snapshot with no memory.
    fn two_member_team() -> TeamSnapshot {
        let alice = build_snapshot(&agent_record("Alice"), MemoryLevel::None, vec![], None);
        let bob = build_snapshot(&agent_record("Bob"), MemoryLevel::None, vec![], None);
        build_team_snapshot(&team_record("My Team"), vec![alice, bob])
    }

    // ── Round-trip tests ──────────────────────────────────────────────────────

    #[test]
    fn json_round_trip_no_memory() {
        let snapshot = two_member_team();
        let bytes = encode_team_snapshot_json(&snapshot).unwrap();
        let parsed = decode_team_snapshot_json(&bytes).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn png_round_trip_no_memory() {
        let snapshot = two_member_team();
        let png_bytes = encode_team_snapshot_png(&snapshot).unwrap();
        assert!(png_bytes.starts_with(b"\x89PNG"), "output must be a PNG");
        let parsed = decode_team_snapshot_png(&png_bytes).unwrap();
        assert_eq!(parsed.team.name, snapshot.team.name);
        assert_eq!(parsed.members.len(), 2);
        assert_eq!(parsed.members[0].definition.name, "Alice Display");
        assert_eq!(parsed.members[1].definition.name, "Bob Display");
    }

    // ── PNG memory guard ──────────────────────────────────────────────────────

    #[test]
    fn png_export_rejected_when_any_member_has_memory() {
        // 2-member team: member[0] has no memory, member[1] has Everything.
        let alice = build_snapshot(&agent_record("Alice"), MemoryLevel::None, vec![], None);
        let entries = vec![AgentSnapshotMemoryEntry {
            slug: "mem/notes".to_string(),
            body: "private notes".to_string(),
        }];
        let bob = build_snapshot(&agent_record("Bob"), MemoryLevel::Everything, entries, None);
        let snapshot = build_team_snapshot(&team_record("Team"), vec![alice, bob]);

        let result = encode_team_snapshot_png(&snapshot);
        assert!(
            result.is_err(),
            "PNG export must fail when any member has memory"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("Cannot write memory to a .team.png"),
            "Error must explain the PNG memory restriction, got: {err}"
        );
        assert!(err.contains("Bob"), "Error must name the offending member");
    }

    #[test]
    fn png_export_rejected_when_none_level_with_nonempty_entries() {
        // Inconsistent state: level == None but entries non-empty on a member.
        // Both the PNG encoder and JSON decoder must reject this via the shared
        // validate_member_memory_consistency helper.
        let mut alice = build_snapshot(&agent_record("Alice"), MemoryLevel::None, vec![], None);
        alice.memory = AgentSnapshotMemory {
            level: MemoryLevel::None,
            entries: vec![AgentSnapshotMemoryEntry {
                slug: "core".to_string(),
                body: "leaked".to_string(),
            }],
        };
        // PNG path
        let snapshot = build_team_snapshot(&team_record("Team"), vec![alice.clone()]);
        assert!(
            encode_team_snapshot_png(&snapshot).is_err(),
            "PNG encoder must reject member with level=None + non-empty entries"
        );
        // JSON decode path: craft the JSON manually and decode it
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_team_snapshot_json(&bytes);
        assert!(
            result.is_err(),
            "JSON decoder must also reject member with level=None + non-empty entries"
        );
    }

    #[test]
    fn png_export_succeeds_when_all_members_have_no_memory() {
        let snapshot = two_member_team();
        assert!(encode_team_snapshot_png(&snapshot).is_ok());
    }

    // ── Validation tests ──────────────────────────────────────────────────────

    #[test]
    fn validate_rejects_wrong_format() {
        let mut snapshot = two_member_team();
        snapshot.format = "not-a-team-snapshot".to_string();
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_team_snapshot_json(&bytes);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Unsupported team snapshot format"));
    }

    #[test]
    fn validate_rejects_wrong_version() {
        let mut snapshot = two_member_team();
        snapshot.version = 99;
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_team_snapshot_json(&bytes);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Unsupported team snapshot version"));
    }

    #[test]
    fn validate_rejects_empty_team_name() {
        let mut snapshot = two_member_team();
        snapshot.team.name = "   ".to_string();
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_team_snapshot_json(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("team.name is empty"));
    }

    #[test]
    fn validate_rejects_zero_members() {
        let alice = build_snapshot(&agent_record("Alice"), MemoryLevel::None, vec![], None);
        let mut snapshot = build_team_snapshot(&team_record("Team"), vec![alice]);
        snapshot.members.clear();
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_team_snapshot_json(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least one member"));
    }

    #[test]
    fn validate_rejects_member_with_wrong_format() {
        let mut snapshot = two_member_team();
        snapshot.members[0].format = "not-an-agent".to_string();
        let bytes = serde_json::to_vec(&snapshot).unwrap();
        let result = decode_team_snapshot_json(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("is invalid"));
    }

    // ── Secret exclusion tests (wrapper-level) ────────────────────────────────
    //
    // Per-member exclusions are already proven in agent_snapshot::tests.
    // These tests assert the team wrapper does not re-introduce any secret
    // field on top of the per-member snapshots.

    fn team_json_string() -> String {
        let bytes = encode_team_snapshot_json(&two_member_team()).unwrap();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn wrapper_does_not_introduce_secret_fields() {
        let json = team_json_string();
        assert!(!json.contains("nsec1secret"), "nsec must not appear");
        assert!(
            !json.contains("auth-tag-secret"),
            "auth_tag value must not appear"
        );
        assert!(!json.contains("API_KEY"), "env var key must not appear");
        assert!(!json.contains("secret123"), "env var value must not appear");
        assert!(
            !json.contains("wss://relay.example.com"),
            "relay_url must not appear"
        );
        assert!(
            !json.contains("SENTINEL_SOURCE_TEAM"),
            "source_team must not appear"
        );
        assert!(
            !json.contains("SENTINEL_SLUG"),
            "source_team_persona_slug must not appear"
        );
        assert!(
            !json.contains("SENTINEL_PERSONA_ID"),
            "persona_id must not appear"
        );
    }

    // ── Structural / shape tests ──────────────────────────────────────────────

    #[test]
    fn member_order_is_preserved() {
        let alice = build_snapshot(&agent_record("Alice"), MemoryLevel::None, vec![], None);
        let bob = build_snapshot(&agent_record("Bob"), MemoryLevel::None, vec![], None);
        let carol = build_snapshot(&agent_record("Carol"), MemoryLevel::None, vec![], None);
        let snapshot = build_team_snapshot(&team_record("Ordered Team"), vec![alice, bob, carol]);

        let bytes = encode_team_snapshot_json(&snapshot).unwrap();
        let parsed = decode_team_snapshot_json(&bytes).unwrap();
        assert_eq!(parsed.members[0].definition.name, "Alice Display");
        assert_eq!(parsed.members[1].definition.name, "Bob Display");
        assert_eq!(parsed.members[2].definition.name, "Carol Display");
    }

    #[test]
    fn description_absent_when_none() {
        let mut record = team_record("No Desc");
        record.description = None;
        let alice = build_snapshot(&agent_record("Alice"), MemoryLevel::None, vec![], None);
        let snapshot = build_team_snapshot(&record, vec![alice]);
        let bytes = encode_team_snapshot_json(&snapshot).unwrap();
        let parsed = decode_team_snapshot_json(&bytes).unwrap();
        assert!(parsed.team.description.is_none());
        let json = String::from_utf8(bytes).unwrap();
        assert!(
            !json.contains("\"description\""),
            "absent description must not serialize"
        );
    }

    #[test]
    fn format_and_version_correct_in_output() {
        let json = team_json_string();
        assert!(
            json.contains("\"buzz-team-snapshot\""),
            "format discriminator must be present"
        );
        assert!(json.contains("\"version\": 1"), "version must be 1");
    }

    #[test]
    fn build_preserves_team_metadata() {
        let snapshot = two_member_team();
        assert_eq!(snapshot.team.name, "My Team");
        assert_eq!(
            snapshot.team.description.as_deref(),
            Some("My Team description")
        );
        assert_eq!(snapshot.members.len(), 2);
        assert_eq!(snapshot.format, FORMAT_DISCRIMINATOR);
        assert_eq!(snapshot.version, FORMAT_VERSION);
    }

    #[test]
    fn decode_rejects_agent_snapshot_json_as_team_snapshot() {
        // An agent snapshot JSON (format: "buzz-agent-snapshot") must NOT
        // parse as a team snapshot — serde rejects missing `team`/`members`
        // fields, or the discriminator check catches it either way.
        use crate::managed_agents::agent_snapshot::{build_snapshot, encode_snapshot_json};
        let agent = build_snapshot(&agent_record("Solo"), MemoryLevel::None, vec![], None);
        let bytes = encode_snapshot_json(&agent).unwrap();
        let result = decode_team_snapshot_json(&bytes);
        assert!(
            result.is_err(),
            "agent snapshot JSON must not parse as team snapshot"
        );
    }

    #[test]
    fn decode_rejects_team_png_as_agent_snapshot() {
        // A .team.png must not silently succeed when passed to the agent PNG
        // decoder — the keyword differs so the agent decoder returns an error.
        use crate::managed_agents::agent_snapshot::decode_snapshot_png;
        let snapshot = two_member_team();
        let png_bytes = encode_team_snapshot_png(&snapshot).unwrap();
        let result = decode_snapshot_png(&png_bytes);
        assert!(result.is_err(), ".team.png must not parse as .agent.png");
    }

    #[test]
    fn decode_team_png_rejects_member_memory() {
        // Craft a memory-bearing .team.png by bypassing the encoder guard:
        // build the manifest with a memory-bearing member, serialize JSON,
        // base64, and write it into a PNG tEXt chunk directly.
        // decode_team_snapshot_png must reject it — defense-in-depth, since
        // the encoder refuses to produce this and any such PNG is malformed
        // or malicious.
        let alice = build_snapshot(&agent_record("Alice"), MemoryLevel::None, vec![], None);
        let entries = vec![AgentSnapshotMemoryEntry {
            slug: "mem/notes".to_string(),
            body: "private".to_string(),
        }];
        let bob = build_snapshot(&agent_record("Bob"), MemoryLevel::Everything, entries, None);
        let snapshot = build_team_snapshot(&team_record("Team"), vec![alice, bob]);

        // Bypass encode_team_snapshot_png's guard — write the chunk directly.
        let json = encode_team_snapshot_json(&snapshot).unwrap();
        let b64 = STANDARD.encode(&json);
        let png = make_png_with_text(PNG_CHUNK_KEYWORD, &b64).unwrap();

        let result = decode_team_snapshot_png(&png);
        assert!(
            result.is_err(),
            "decode must reject a memory-bearing .team.png"
        );
        assert!(result
            .unwrap_err()
            .contains("Cannot write memory to a .team.png"));
    }
}
