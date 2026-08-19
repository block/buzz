use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use super::storage::{atomic_write_json_restricted, backup_invalid_store, managed_agents_base_dir};
use crate::app_state::AppState;

const STORE_FILENAME: &str = "registered-agent-references.json";
const AGENTS_DATA_CHANGED_EVENT: &str = "agents-data-changed";
const LABEL_LIMIT_BYTES: usize = 80;
const ROLE_SUMMARY_LIMIT_BYTES: usize = 240;

/// A keyless reference to an already-existing agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredAgentReference {
    /// The referenced agent public key as normalized 64-byte lowercase hex.
    pub pubkey: String,
    /// Optional user-facing label for the reference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Optional short description of the agent's role.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_summary: Option<String>,
    /// Creation timestamp in ISO-8601 UTC form.
    pub created_at: String,
    /// Last update timestamp in ISO-8601 UTC form.
    pub updated_at: String,
}

/// Request payload for registering or updating an existing agent reference.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterAgentReferenceRequest {
    /// Agent public key, accepted with leading/trailing whitespace and mixed case.
    pub pubkey: String,
    /// Optional label; blank strings are stored as `None`.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional role summary; blank strings are stored as `None`.
    #[serde(default)]
    pub role_summary: Option<String>,
}

fn store_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join(STORE_FILENAME))
}

fn normalize_pubkey(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.len() != 64 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid public key".to_string());
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn normalize_optional(
    value: Option<String>,
    limit: usize,
    field: &str,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > limit {
        return Err(format!("{field} must be at most {limit} UTF-8 bytes"));
    }
    Ok(Some(trimmed.to_string()))
}

fn load_from_path(path: &Path) -> Result<Vec<RegisteredAgentReference>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read registered agent references: {error}"))?;
    let mut refs: Vec<RegisteredAgentReference> =
        serde_json::from_str(&content).map_err(|error| {
            backup_invalid_store(path);
            format!("failed to parse registered agent references (preserved as .invalid): {error}")
        })?;
    refs.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    if let Some(duplicate) = refs
        .windows(2)
        .find(|pair| pair[0].pubkey == pair[1].pubkey)
    {
        backup_invalid_store(path);
        return Err(format!(
            "duplicate registered agent pubkey {} (preserved as .invalid)",
            duplicate[0].pubkey
        ));
    }
    Ok(refs)
}

fn save_to_path(path: &Path, refs: &[RegisteredAgentReference]) -> Result<(), String> {
    let mut refs = refs.to_vec();
    refs.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    let payload = serde_json::to_vec_pretty(&refs)
        .map_err(|error| format!("failed to serialize registered agent references: {error}"))?;
    atomic_write_json_restricted(path, &payload)
}

/// Load all registered existing-agent references.
#[tauri::command]
pub fn list_registered_agent_references(
    app: AppHandle,
) -> Result<Vec<RegisteredAgentReference>, String> {
    load_from_path(&store_path(&app)?)
}

/// Register or update a keyless reference to an existing agent identity.
#[tauri::command]
pub fn register_existing_agent_reference(
    input: RegisterAgentReferenceRequest,
    app: AppHandle,
) -> Result<RegisteredAgentReference, String> {
    let pubkey = normalize_pubkey(&input.pubkey)?;
    let label = normalize_optional(input.label, LABEL_LIMIT_BYTES, "label")?;
    let role_summary =
        normalize_optional(input.role_summary, ROLE_SUMMARY_LIMIT_BYTES, "roleSummary")?;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    if super::storage::managed_agent_record_exists(&app, &pubkey)? {
        return Err(format!("agent {pubkey} is already a managed agent"));
    }
    let path = store_path(&app)?;
    let mut refs = load_from_path(&path)?;
    let now = crate::util::now_iso();
    let reference = match refs.iter_mut().find(|reference| reference.pubkey == pubkey) {
        Some(existing) => {
            existing.label = label;
            existing.role_summary = role_summary;
            existing.updated_at = now;
            existing.clone()
        }
        None => {
            let reference = RegisteredAgentReference {
                pubkey,
                label,
                role_summary,
                created_at: now.clone(),
                updated_at: now,
            };
            refs.push(reference.clone());
            reference
        }
    };
    save_to_path(&path, &refs)?;
    let _ = app.emit(AGENTS_DATA_CHANGED_EVENT, ());
    Ok(reference)
}

/// Remove a registered existing-agent reference by public key.
#[tauri::command]
pub fn unregister_existing_agent_reference(pubkey: String, app: AppHandle) -> Result<(), String> {
    let pubkey = normalize_pubkey(&pubkey)?;
    let state = app.state::<AppState>();
    let _store_guard = state
        .managed_agents_store_lock
        .lock()
        .map_err(|error| error.to_string())?;
    let path = store_path(&app)?;
    let mut refs = load_from_path(&path)?;
    let initial_len = refs.len();
    refs.retain(|reference| reference.pubkey != pubkey);
    if refs.len() == initial_len {
        return Err(format!("agent {pubkey} not found"));
    }
    save_to_path(&path, &refs)?;
    let _ = app.emit(AGENTS_DATA_CHANGED_EVENT, ());
    Ok(())
}

/// Require a managed-agent record before any lifecycle/config/delete side
/// effect. Registered references and unknown pubkeys both fail closed.
pub(crate) fn reject_registered_reference_target(
    app: &AppHandle,
    pubkey: &str,
) -> Result<(), String> {
    let normalized = normalize_pubkey(pubkey).unwrap_or_else(|_| pubkey.to_string());
    reject_registered_reference_target_at_path(
        &super::storage::managed_agents_store_path(app)?,
        &normalized,
    )
}

fn reject_registered_reference_target_at_path(path: &Path, pubkey: &str) -> Result<(), String> {
    if super::storage::managed_agent_record_exists_at_path(path, pubkey)? {
        Ok(())
    } else {
        Err(format!("agent {pubkey} not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    const PUBKEY_A_UPPER: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const PUBKEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PUBKEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const COMMAND_TARGETS_SOURCE: &str = include_str!("../commands/agent_registered_targets.rs");
    const SETTINGS_SOURCE: &str = include_str!("../commands/agent_settings.rs");
    const RUNTIME_SOURCE: &str = include_str!("runtime_commands.rs");

    fn request(
        pubkey: &str,
        label: Option<&str>,
        role_summary: Option<&str>,
    ) -> RegisterAgentReferenceRequest {
        RegisterAgentReferenceRequest {
            pubkey: pubkey.to_string(),
            label: label.map(str::to_string),
            role_summary: role_summary.map(str::to_string),
        }
    }

    #[test]
    fn normalizes_pubkey_and_blank_optionals() {
        let normalized = normalize_pubkey(&format!("  {PUBKEY_A_UPPER}  ")).unwrap();
        assert_eq!(normalized, PUBKEY_A);
        assert_eq!(
            normalize_optional(Some("   ".to_string()), LABEL_LIMIT_BYTES, "label").unwrap(),
            None
        );
        assert_eq!(
            normalize_optional(Some("  hello  ".to_string()), LABEL_LIMIT_BYTES, "label").unwrap(),
            Some("hello".to_string())
        );
    }

    #[test]
    fn rejects_non_exact_ascii_hex_pubkeys_and_overlong_fields() {
        assert_eq!(normalize_pubkey("abc").unwrap_err(), "invalid public key");
        assert!(normalize_pubkey(
            "g000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        assert!(normalize_optional(Some("é".repeat(41)), LABEL_LIMIT_BYTES, "label").is_err());
        assert!(normalize_optional(
            Some("x".repeat(241)),
            ROLE_SUMMARY_LIMIT_BYTES,
            "roleSummary"
        )
        .is_err());
    }

    #[test]
    fn save_load_sorts_by_pubkey_and_uses_restricted_mode() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STORE_FILENAME);
        let refs = vec![
            RegisteredAgentReference {
                pubkey: PUBKEY_B.to_string(),
                label: None,
                role_summary: None,
                created_at: "2026-01-02T00:00:00Z".to_string(),
                updated_at: "2026-01-02T00:00:00Z".to_string(),
            },
            RegisteredAgentReference {
                pubkey: PUBKEY_A.to_string(),
                label: Some("A".to_string()),
                role_summary: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];

        save_to_path(&path, &refs).unwrap();
        let loaded = load_from_path(&path).unwrap();

        assert_eq!(
            loaded.iter().map(|r| r.pubkey.as_str()).collect::<Vec<_>>(),
            vec![PUBKEY_A, PUBKEY_B]
        );
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn malformed_json_fails_closed_and_preserves_invalid_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STORE_FILENAME);
        let bytes = b"{ definitely not json";
        fs::write(&path, bytes).unwrap();

        let error = load_from_path(&path).unwrap_err();

        assert!(error.contains("failed to parse registered agent references"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            fs::read(path.with_extension("json.invalid")).unwrap(),
            bytes
        );
    }

    #[test]
    fn duplicate_pubkeys_fail_closed_and_preserve_invalid_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STORE_FILENAME);
        let refs = vec![
            RegisteredAgentReference {
                pubkey: PUBKEY_A.to_string(),
                label: Some("first".to_string()),
                role_summary: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            },
            RegisteredAgentReference {
                pubkey: PUBKEY_A.to_string(),
                label: Some("duplicate".to_string()),
                role_summary: None,
                created_at: "2026-01-02T00:00:00Z".to_string(),
                updated_at: "2026-01-02T00:00:00Z".to_string(),
            },
        ];
        let bytes = serde_json::to_vec_pretty(&refs).unwrap();
        fs::write(&path, &bytes).unwrap();

        let error = load_from_path(&path).unwrap_err();

        assert!(error.contains("duplicate registered agent pubkey"));
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(
            fs::read(path.with_extension("json.invalid")).unwrap(),
            bytes
        );
    }

    #[test]
    fn update_preserves_created_at_and_replaces_optionals() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(STORE_FILENAME);
        let mut refs = Vec::new();
        let first = upsert_for_test(
            &path,
            request(PUBKEY_A, Some("one"), Some("old")),
            &mut refs,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        let second = upsert_for_test(
            &path,
            request(PUBKEY_A_UPPER, Some("two"), None),
            &mut refs,
            "2026-01-02T00:00:00Z",
        )
        .unwrap();

        assert_eq!(first.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(second.created_at, first.created_at);
        assert_eq!(second.updated_at, "2026-01-02T00:00:00Z");
        assert_eq!(second.label.as_deref(), Some("two"));
        assert_eq!(second.role_summary, None);
    }

    #[test]
    fn request_denies_unknown_fields() {
        let json = format!(r#"{{"pubkey":"{PUBKEY_A}","label":"hi","privateKeyNsec":"nope"}}"#);
        let error = serde_json::from_str::<RegisterAgentReferenceRequest>(&json).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    fn upsert_for_test(
        path: &Path,
        input: RegisterAgentReferenceRequest,
        refs: &mut Vec<RegisteredAgentReference>,
        now: &str,
    ) -> Result<RegisteredAgentReference, String> {
        let pubkey = normalize_pubkey(&input.pubkey)?;
        let label = normalize_optional(input.label, LABEL_LIMIT_BYTES, "label")?;
        let role_summary =
            normalize_optional(input.role_summary, ROLE_SUMMARY_LIMIT_BYTES, "roleSummary")?;
        let reference = match refs.iter_mut().find(|reference| reference.pubkey == pubkey) {
            Some(existing) => {
                existing.label = label;
                existing.role_summary = role_summary;
                existing.updated_at = now.to_string();
                existing.clone()
            }
            None => {
                let reference = RegisteredAgentReference {
                    pubkey,
                    label,
                    role_summary,
                    created_at: now.to_string(),
                    updated_at: now.to_string(),
                };
                refs.push(reference.clone());
                reference
            }
        };
        save_to_path(path, refs)?;
        Ok(reference)
    }

    /// Extract one small command body so the regression tests verify the real
    /// production routing as well as the path-based ownership decision. These
    /// commands deliberately contain no nested brace-bearing literals before
    /// their ownership check.
    fn command_body<'a>(source: &'a str, name: &str) -> &'a str {
        let signature = format!("fn {name}(");
        let start = source
            .find(&signature)
            .unwrap_or_else(|| panic!("missing command {name}"));
        let body_start = source[start..]
            .find('{')
            .map(|offset| start + offset)
            .unwrap_or_else(|| panic!("missing body for command {name}"));
        let mut depth = 0usize;
        for (offset, byte) in source[body_start..].bytes().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[body_start..=body_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("unterminated body for command {name}")
    }

    fn assert_before(body: &str, first: &str, second: &str) {
        let first_index = body
            .find(first)
            .unwrap_or_else(|| panic!("missing preflight `{first}` in {body}"));
        let second_index = body
            .find(second)
            .unwrap_or_else(|| panic!("missing mutation/delegation `{second}` in {body}"));
        assert!(
            first_index < second_index,
            "`{first}` must precede `{second}`"
        );
    }

    fn assert_production_command_is_guarded(target: &str) {
        match target {
            "start_managed_agent"
            | "stop_managed_agent"
            | "update_managed_agent"
            | "delete_managed_agent" => assert_before(
                command_body(COMMAND_TARGETS_SOURCE, target),
                "reject_registered_reference_target",
                &format!("{target}_unchecked"),
            ),
            "set_managed_agent_start_on_app_launch" | "set_managed_agent_auto_restart" => {
                assert_before(
                    command_body(SETTINGS_SOURCE, target),
                    "reject_registered_reference_target",
                    "spawn_blocking",
                );
            }
            "put_managed_agent_runtime_lifecycle" => assert_before(
                command_body(RUNTIME_SOURCE, target),
                "reject_registered_reference_target",
                "app.state::<AppState>()",
            ),
            "start_managed_agent_runtime" => {
                assert!(command_body(RUNTIME_SOURCE, target)
                    .contains("start_managed_agent_runtime_pair_lazy"));
                assert_before(
                    command_body(RUNTIME_SOURCE, "start_pair"),
                    "reject_registered_reference_target",
                    "app.state::<AppState>()",
                );
            }
            "stop_managed_agent_runtime" => assert_before(
                command_body(RUNTIME_SOURCE, target),
                "reject_registered_reference_target",
                "app.state::<AppState>()",
            ),
            "restart_managed_agent_runtime" => assert_before(
                command_body(RUNTIME_SOURCE, target),
                "stop_managed_agent_runtime",
                "start_pair",
            ),
            _ => panic!("unmapped registered-reference command target {target}"),
        }
    }

    fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        fn visit(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Vec<u8>)>) {
            let mut entries = fs::read_dir(dir)
                .unwrap()
                .map(|entry| entry.unwrap())
                .collect::<Vec<_>>();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    visit(root, &path, out);
                } else {
                    out.push((
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(path).unwrap(),
                    ));
                }
            }
        }
        let mut snapshot = Vec::new();
        visit(root, root, &mut snapshot);
        snapshot
    }

    fn assert_registered_reference_command_fails_closed(target: &str) {
        let temp = tempfile::tempdir().unwrap();
        let agents_dir = temp.path().join("agents");
        fs::create_dir_all(agents_dir.join("agent-pids")).unwrap();
        let managed_store = agents_dir.join("managed-agents.json");
        let registered_store = agents_dir.join(STORE_FILENAME);

        // The target exists only in the keyless reference store. The managed
        // store contains an unrelated record with deliberately stale runtime
        // metadata, matching the state that used to be synchronized before a
        // missing-target error was returned.
        let unrelated = serde_json::json!({
            "pubkey": PUBKEY_B,
            "name": "unrelated-exited-runtime",
            "relay_url": "wss://relay.example",
            "acp_command": "buzz-acp",
            "agent_command": "goose",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": "",
            "runtime_pid": 424242,
            "last_error": "stale exited child",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        });
        fs::write(
            &managed_store,
            serde_json::to_vec_pretty(&vec![unrelated]).unwrap(),
        )
        .unwrap();
        fs::write(
            &registered_store,
            serde_json::to_vec_pretty(&vec![RegisteredAgentReference {
                pubkey: PUBKEY_A.to_string(),
                label: Some("external".to_string()),
                role_summary: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
            }])
            .unwrap(),
        )
        .unwrap();

        // Sentinels cover every side-effect class at issue: event emission,
        // keyring writes/deletes, in-memory lifecycle state, runtime receipts,
        // and process identity. A command reaching its mutation body would
        // necessarily alter at least one of these or the managed store.
        fs::write(agents_dir.join("event-sentinel"), b"0 events").unwrap();
        fs::write(
            agents_dir.join("keyring-sentinel"),
            b"agent:b = nsec-sentinel",
        )
        .unwrap();
        fs::write(
            agents_dir.join("runtime-state-sentinel"),
            br#"{"pubkey":"bbbb","lifecycle":"exited","pid":424242}"#,
        )
        .unwrap();
        fs::write(
            agents_dir.join("agent-pids").join("stale.json"),
            br#"{"pid":424242,"status":"exited"}"#,
        )
        .unwrap();

        let before = snapshot_tree(temp.path());
        let error = reject_registered_reference_target_at_path(&managed_store, PUBKEY_A)
            .expect_err("registered reference must not authorize a managed command");
        assert_eq!(error, format!("agent {PUBKEY_A} not found"));
        assert_eq!(snapshot_tree(temp.path()), before, "{target} mutated state");
        assert_production_command_is_guarded(target);
    }

    macro_rules! fail_closed_command_test {
        ($name:ident, $target:literal) => {
            #[test]
            fn $name() {
                assert_registered_reference_command_fails_closed($target);
            }
        };
    }

    fail_closed_command_test!(
        registered_agent_references_start_managed_agent_fails_closed,
        "start_managed_agent"
    );
    fail_closed_command_test!(
        registered_agent_references_stop_managed_agent_fails_closed,
        "stop_managed_agent"
    );
    fail_closed_command_test!(
        registered_agent_references_start_runtime_fails_closed,
        "start_managed_agent_runtime"
    );
    fail_closed_command_test!(
        registered_agent_references_stop_runtime_fails_closed,
        "stop_managed_agent_runtime"
    );
    fail_closed_command_test!(
        registered_agent_references_restart_runtime_fails_closed,
        "restart_managed_agent_runtime"
    );
    fail_closed_command_test!(
        registered_agent_references_lifecycle_observer_write_fails_closed,
        "put_managed_agent_runtime_lifecycle"
    );
    fail_closed_command_test!(
        registered_agent_references_start_on_launch_fails_closed,
        "set_managed_agent_start_on_app_launch"
    );
    fail_closed_command_test!(
        registered_agent_references_auto_restart_fails_closed,
        "set_managed_agent_auto_restart"
    );
    fail_closed_command_test!(
        registered_agent_references_update_fails_closed,
        "update_managed_agent"
    );
    fail_closed_command_test!(
        registered_agent_references_delete_fails_closed,
        "delete_managed_agent"
    );
}
