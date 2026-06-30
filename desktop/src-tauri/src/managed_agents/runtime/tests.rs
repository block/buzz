use crate::managed_agents::known_acp_runtime;

// ── buffer_contains_identifier tests ────────────────────────────────────

#[test]
fn identifier_prefix_does_not_match_longer_id() {
    // DMG identifier should NOT match inside a dev desktop's config JSON.
    let buf = br#""identifier":"xyz.block.buzz.app.dev""#;
    let id = b"xyz.block.buzz.app";
    assert!(!super::buffer_contains_identifier(buf, id));
}

#[test]
fn identifier_prefix_does_not_match_worktree_slug() {
    // Main dev identifier should NOT match inside a worktree desktop's buffer.
    let buf = br#""identifier":"xyz.block.buzz.app.dev.my-branch""#;
    let id = b"xyz.block.buzz.app.dev";
    assert!(!super::buffer_contains_identifier(buf, id));
}

#[test]
fn identifier_exact_match_with_quote_boundary() {
    // Exact match followed by closing quote — should match.
    let buf = br#""identifier":"xyz.block.buzz.app.dev""#;
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(buf, id));
}

#[test]
fn identifier_match_with_null_boundary() {
    // In KERN_PROCARGS2, entries are null-delimited.
    let mut buf = b"BUZZ_MANAGED_AGENT=xyz.block.buzz.app.dev".to_vec();
    buf.push(0);
    buf.extend_from_slice(b"OTHER_VAR=value");
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(&buf, id));
}

#[test]
fn identifier_exact_match_at_end_of_buffer() {
    // Exact match with end-of-buffer as the boundary — Thufir's case 1.
    let buf = b"xyz.block.buzz.app.dev";
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(buf, id));
}

#[test]
fn longer_id_matches_when_short_prefix_also_present() {
    // Searching for the longer ID finds it even when a shorter prefix token
    // appears earlier — Thufir's "longer-of-prefix must match" case.
    let mut buf = b"xyz.block.buzz.app".to_vec();
    buf.push(0);
    buf.extend_from_slice(br#""identifier":"xyz.block.buzz.app.dev""#);
    let id = b"xyz.block.buzz.app.dev";
    assert!(super::buffer_contains_identifier(&buf, id));
}

#[test]
fn identifier_empty_returns_false() {
    let buf = b"anything";
    assert!(!super::buffer_contains_identifier(buf, b""));
}

// ── marker_entry tests ──────────────────────────────────────────────────

#[test]
fn marker_entry_is_namespaced_by_instance_id() {
    // The spawn stamp and the sweep matcher must produce identical bytes;
    // both go through buzz_marker_entry, so this pins the on-the-wire
    // format and guards against a dev build (`...app.dev`) matching a
    // release build's (`...app`) agents.
    assert_eq!(
        super::buzz_marker_entry("xyz.block.buzz.app"),
        b"BUZZ_MANAGED_AGENT=xyz.block.buzz.app".to_vec()
    );
    assert_ne!(
        super::buzz_marker_entry("xyz.block.buzz.app"),
        super::buzz_marker_entry("xyz.block.buzz.app.dev")
    );
}

#[test]
fn buzz_agent_has_mcp_hooks() {
    let p = known_acp_runtime("buzz-agent").expect("should resolve");
    assert!(p.mcp_hooks);
    assert_eq!(p.mcp_command, Some("buzz-dev-mcp"));
}

#[test]
fn buzz_agent_resolved_via_path() {
    assert!(known_acp_runtime("/usr/local/bin/buzz-agent").is_some_and(|p| p.mcp_hooks));
}

#[test]
fn goose_has_no_mcp_hooks() {
    let p = known_acp_runtime("goose").expect("should resolve");
    assert!(!p.mcp_hooks);
    assert_eq!(p.mcp_command, None);
}

#[test]
fn unknown_command_returns_none() {
    assert!(known_acp_runtime("custom-agent").is_none());
}

// ── build_respond_to_env tests ───────────────────────────────────────

use super::build_respond_to_env;
use crate::managed_agents::types::{ManagedAgentRecord, RespondTo};

/// Construct a minimal record fixture for env-building tests. Only the
/// fields read by `build_respond_to_env` matter here.
fn fixture(
    respond_to: RespondTo,
    allowlist: Vec<String>,
    auth_tag: Option<String>,
) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: "p".into(),
        name: "n".into(),
        persona_id: None,
        private_key_nsec: "nsec1fake".into(),
        auth_tag,
        relay_url: "ws://localhost:3000".into(),
        avatar_url: None,
        acp_command: "buzz-acp".into(),
        agent_command: "goose".into(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 320,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        mcp_toolsets: None,
        env_vars: std::collections::BTreeMap::new(),
        start_on_app_launch: false,
        runtime_pid: None,
        backend: Default::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: "now".into(),
        updated_at: "now".into(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        respond_to,
        respond_to_allowlist: allowlist,
        relay_mesh: None,
    }
}

#[test]
fn build_env_owner_only_sets_mode_and_removes_others() {
    let owner = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let agent_pubkey = agent.public_key().to_hex();
    let auth_tag = super::managed_agent_auth_tag_for_owner(&owner, &agent_pubkey)
        .unwrap()
        .unwrap();
    let mut rec = fixture(RespondTo::OwnerOnly, vec![], Some(auth_tag));
    rec.pubkey = agent_pubkey;
    let owner_hex = owner.public_key().to_hex();
    let (set, remove) = build_respond_to_env(&rec, Some(&owner_hex)).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("owner-only")
    );
    assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    // auth_tag is present → no AGENT_OWNER fallback fires.
    assert!(remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_stale_auth_tag_emits_current_owner() {
    let previous_owner = nostr::Keys::generate();
    let current_owner = nostr::Keys::generate();
    let agent = nostr::Keys::generate();
    let agent_pubkey = agent.public_key().to_hex();
    let stale_auth_tag = super::managed_agent_auth_tag_for_owner(&previous_owner, &agent_pubkey)
        .unwrap()
        .unwrap();
    let mut rec = fixture(RespondTo::OwnerOnly, vec![], Some(stale_auth_tag));
    rec.pubkey = agent_pubkey;
    let current_owner_hex = current_owner.public_key().to_hex();
    let (set, remove) = build_respond_to_env(&rec, Some(&current_owner_hex)).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_AGENT_OWNER").map(String::as_str),
        Some(current_owner_hex.as_str())
    );
    assert!(!remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_allowlist_sets_both_envs_and_joins() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let rec = fixture(
        RespondTo::Allowlist,
        vec![a.clone(), b.clone()],
        Some("tag".into()),
    );
    let (set, _remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("allowlist")
    );
    assert_eq!(
        set_map
            .get("BUZZ_ACP_RESPOND_TO_ALLOWLIST")
            .map(String::as_str),
        Some(format!("{a},{b}").as_str()),
    );
}

#[test]
fn build_env_anyone_omits_allowlist_var() {
    let rec = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    let (set, remove) = build_respond_to_env(&rec, Some("owner")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_RESPOND_TO").map(String::as_str),
        Some("anyone")
    );
    assert!(!set_map.contains_key("BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
    assert!(remove.contains(&"BUZZ_ACP_RESPOND_TO_ALLOWLIST"));
}

#[test]
fn build_env_legacy_record_without_auth_tag_emits_agent_owner() {
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (set, remove) = build_respond_to_env(&rec, Some("ownerhex")).unwrap();
    let set_map: std::collections::HashMap<_, _> = set.into_iter().collect();
    assert_eq!(
        set_map.get("BUZZ_ACP_AGENT_OWNER").map(String::as_str),
        Some("ownerhex")
    );
    assert!(!remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_legacy_record_without_owner_hex_removes_agent_owner() {
    // No owner available to forward → make sure we don't inherit a leaked
    // env var from the parent.
    let rec = fixture(RespondTo::OwnerOnly, vec![], None);
    let (_set, remove) = build_respond_to_env(&rec, None).unwrap();
    assert!(remove.contains(&"BUZZ_ACP_AGENT_OWNER"));
}

#[test]
fn build_env_rejects_corrupted_allowlist() {
    let rec = fixture(
        RespondTo::Allowlist,
        vec!["not-hex".into()],
        Some("tag".into()),
    );
    assert!(build_respond_to_env(&rec, Some("owner")).is_err());
}

#[test]
fn build_env_rejects_empty_allowlist_in_allowlist_mode() {
    let rec = fixture(RespondTo::Allowlist, vec![], Some("tag".into()));
    let err = build_respond_to_env(&rec, Some("owner")).unwrap_err();
    assert!(err.contains("at least one pubkey"));
}

// ── persona fixture helpers ─────────────────────────────────────────

fn persona_with_provider(
    id: &str,
    prompt: &str,
    model: Option<&str>,
    provider: Option<&str>,
) -> crate::managed_agents::PersonaRecord {
    crate::managed_agents::PersonaRecord {
        id: id.to_string(),
        display_name: id.to_string(),
        avatar_url: None,
        system_prompt: prompt.to_string(),
        runtime: None,
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: std::collections::BTreeMap::new(),
        created_at: "2026-06-09T00:00:00Z".to_string(),
        updated_at: "2026-06-09T00:00:00Z".to_string(),
    }
}

// ── persona pin/refresh acceptance (Phase 4) ────────────────────────────
//
// The full lifecycle Will specified: create from P0, edit P0→P1 (env_vars
// included), restart stays pinned to P0, delete+respawn refreshes to P1. We
// exercise it at the pure seams that `create_managed_agent` and
// `build_managed_agent_summary` are built from: `persona_snapshot` (what create
// writes onto the record) and `persona_drift_state` (the Agents-menu badge).
// The env_var assertions are load-bearing — the credential pin is the field
// that would silently leak on restart if spawn re-read the live persona.

use crate::managed_agents::persona_events::persona_snapshot;
use std::collections::BTreeMap;

/// Apply a persona snapshot onto a record, mirroring `create_managed_agent`:
/// snapshotted prompt/model/provider/env_vars/source_version are pinned, with
/// the system_prompt unwrapped (the persona always carries one).
fn pin_persona(record: &mut ManagedAgentRecord, persona: &crate::managed_agents::PersonaRecord) {
    let snapshot = persona_snapshot(persona, &record.env_vars);
    record.persona_id = Some(persona.id.clone());
    record.system_prompt = snapshot.system_prompt;
    record.model = snapshot.model;
    record.provider = snapshot.provider;
    record.env_vars = snapshot.env_vars;
    record.persona_source_version = Some(snapshot.source_version);
}

fn persona_v(id: &str, prompt: &str, env: &[(&str, &str)]) -> crate::managed_agents::PersonaRecord {
    let mut p = persona_with_provider(id, prompt, Some("model-v"), Some("anthropic"));
    p.env_vars = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    p
}

#[test]
fn create_pins_full_persona_snapshot_including_env_vars() {
    let p0 = persona_v("p", "prompt-v0", &[("ANTHROPIC_API_KEY", "key-v0")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &p0);

    assert_eq!(record.system_prompt.as_deref(), Some("prompt-v0"));
    assert_eq!(record.provider.as_deref(), Some("anthropic"));
    assert_eq!(
        record.env_vars.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("key-v0"),
        "create must pin persona env_vars — the credential pin"
    );
    assert!(record.persona_source_version.is_some());
}

#[test]
fn restart_after_persona_edit_stays_pinned_to_old_snapshot() {
    // Create from P0.
    let p0 = persona_v("p", "prompt-v0", &[("ANTHROPIC_API_KEY", "key-v0")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &p0);

    // Edit the persona to P1 (prompt + credential change). Restart reuses the
    // SAME record — nothing rewrites the snapshot — so spawn reads P0 fields.
    let p1 = persona_v("p", "prompt-v1", &[("ANTHROPIC_API_KEY", "key-v1")]);

    assert_eq!(
        record.system_prompt.as_deref(),
        Some("prompt-v0"),
        "restart must keep the pinned prompt"
    );
    assert_eq!(
        record.env_vars.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("key-v0"),
        "restart must NOT pick up the edited credential — the CRITICAL"
    );

    // The badge flips: the record's snapshot now lags the edited persona.
    let (out_of_date, orphaned) = super::persona_drift_state(&record, std::slice::from_ref(&p1));
    assert!(
        out_of_date,
        "edited persona must mark the instance out of date"
    );
    assert!(!orphaned);
}

#[test]
fn respawn_after_persona_edit_refreshes_to_new_snapshot() {
    let p0 = persona_v("p", "prompt-v0", &[("ANTHROPIC_API_KEY", "key-v0")]);
    let p1 = persona_v("p", "prompt-v1", &[("ANTHROPIC_API_KEY", "key-v1")]);

    // Respawn = delete the old record + create a fresh one. create re-snapshots
    // the now-current persona (P1).
    let mut respawned = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut respawned, &p1);

    assert_eq!(respawned.system_prompt.as_deref(), Some("prompt-v1"));
    assert_eq!(
        respawned
            .env_vars
            .get("ANTHROPIC_API_KEY")
            .map(String::as_str),
        Some("key-v1"),
        "respawn must refresh the credential to the edited persona"
    );

    // Now pinned to current persona → not out of date.
    let (out_of_date, orphaned) = super::persona_drift_state(&respawned, std::slice::from_ref(&p1));
    assert!(!out_of_date, "respawn pins to current persona — no drift");
    assert!(!orphaned);

    // Sanity: P0 differs from P1, so a record still pinned to P0 would drift.
    let mut stale = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut stale, &p0);
    assert!(super::persona_drift_state(&stale, std::slice::from_ref(&p1)).0);
}

#[test]
fn agent_env_overrides_win_over_persona_env_in_snapshot() {
    // Agent-level env_vars (input.env_vars) layer over persona env on collision,
    // matching spawn precedence (persona env < agent env).
    let persona = persona_v("p", "prompt", &[("ANTHROPIC_API_KEY", "persona-key")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    record.env_vars = BTreeMap::from([("ANTHROPIC_API_KEY".to_string(), "agent-key".to_string())]);
    pin_persona(&mut record, &persona);

    assert_eq!(
        record.env_vars.get("ANTHROPIC_API_KEY").map(String::as_str),
        Some("agent-key"),
        "agent override must win over persona env"
    );
}

#[test]
fn deleted_persona_is_orphaned_not_out_of_date() {
    let p0 = persona_v("p", "prompt-v0", &[("KEY", "v0")]);
    let mut record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    pin_persona(&mut record, &p0);

    // Persona no longer in the catalog → orphaned, never out of date (no
    // current persona to respawn into).
    let (out_of_date, orphaned) = super::persona_drift_state(&record, &[]);
    assert!(!out_of_date);
    assert!(orphaned);
}

#[test]
fn non_persona_agent_never_drifts() {
    // A hand-built agent (no persona_id) has nothing to drift from.
    let record = fixture(RespondTo::Anyone, vec![], Some("tag".into()));
    assert_eq!(record.persona_id, None);
    let (out_of_date, orphaned) = super::persona_drift_state(&record, &[]);
    assert!(!out_of_date);
    assert!(!orphaned);
}

use super::runtime_metadata_env_vars;

#[test]
fn runtime_metadata_env_vars_injects_model_and_provider() {
    let vars = runtime_metadata_env_vars(
        Some("GOOSE_MODEL"),
        Some("GOOSE_PROVIDER"),
        false,
        Some("gpt-4o"),
        Some("openai"),
    );
    assert_eq!(
        vars,
        vec![("GOOSE_MODEL", "gpt-4o"), ("GOOSE_PROVIDER", "openai")]
    );
}

#[test]
fn runtime_metadata_env_vars_skips_provider_when_locked() {
    let vars = runtime_metadata_env_vars(
        None, // claude has no model_env_var
        None, // claude has no provider_env_var
        true, // provider_locked = true
        Some("claude-opus-4-7"),
        Some("anthropic"),
    );
    assert!(vars.is_empty());
}

#[test]
fn runtime_metadata_env_vars_injects_model_even_with_acp_model_switching() {
    // buzz-agent has supports_acp_model_switching=true but we still inject
    // the model env var because ACP model switching is post-bootstrap
    let vars = runtime_metadata_env_vars(
        Some("BUZZ_AGENT_MODEL"),
        Some("BUZZ_AGENT_PROVIDER"),
        false,
        Some("goose-claude-4-6-opus"),
        Some("databricks"),
    );
    assert_eq!(
        vars,
        vec![
            ("BUZZ_AGENT_MODEL", "goose-claude-4-6-opus"),
            ("BUZZ_AGENT_PROVIDER", "databricks"),
        ]
    );
}

// ── name_matches_known_binary / name_matches_interpreter tests ───────────

#[test]
fn name_matches_known_binary_rejects_node() {
    // `node` must NOT be in KNOWN_AGENT_BINARIES — adding it there would
    // sweep all node processes on the machine regardless of ownership.
    assert!(!super::name_matches_known_binary("node"));
}

#[test]
fn name_matches_interpreter_accepts_node() {
    // `node` IS a known script interpreter and must be recognized.
    assert!(super::name_matches_interpreter("node"));
}

#[test]
fn name_matches_interpreter_rejects_unknown() {
    // Interpreters not in KNOWN_SCRIPT_INTERPRETERS must not match.
    assert!(!super::name_matches_interpreter("python3"));
    assert!(!super::name_matches_interpreter("deno"));
    assert!(!super::name_matches_interpreter("bun"));
}

#[test]
fn name_matches_interpreter_rejects_node_prefix() {
    // A name that starts with "node" but is longer must not match —
    // exact equality is required to avoid false positives.
    assert!(!super::name_matches_interpreter("node_modules"));
    assert!(!super::name_matches_interpreter("nodejs"));
    assert!(!super::name_matches_interpreter("node-gyp"));
}

// ── PGID-based orphan sweep tests ───────────────────────────────────────

/// Validates the kernel invariant that the orphan sweep PGID fix relies on:
/// a grandchild process inherits the PGID of the process group leader (the
/// harness), so checking PGID membership in `skip_pids` correctly identifies
/// live descendants even when their ppid is an intermediate process (e.g.
/// goose) rather than the harness itself.
#[cfg(unix)]
#[test]
fn grandchild_inherits_pgid_of_process_group_leader() {
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    // Spawn a "harness" process in its own process group (mirrors
    // `command.process_group(0)` in the real spawn path). The harness
    // spawns an intermediate child which in turn spawns a grandchild.
    // This mirrors the real tree: buzz-acp → goose → buzz-dev-mcp.
    //
    // The intermediate `sh` uses exec to replace itself with another sh
    // that backgrounds the grandchild, so the grandchild's ppid is the
    // intermediate (not the harness).
    let mut harness = {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sh -c 'sleep 10 & echo $!' & wait $!"])
            .stdout(std::process::Stdio::piped())
            .process_group(0);
        cmd.spawn().expect("spawn harness")
    };

    // Read the grandchild PID from stdout.
    use std::io::BufRead;
    let stdout = harness.stdout.take().unwrap();
    let reader = std::io::BufReader::new(stdout);
    let grandchild_pid: i32 = reader
        .lines()
        .next()
        .expect("should get a line")
        .expect("should read line")
        .trim()
        .parse()
        .expect("should parse grandchild PID");

    let harness_pid = harness.id() as i32;

    // The harness is the process group leader (PGID == its own PID).
    let harness_pgid = unsafe { libc::getpgid(harness_pid) };
    assert_eq!(
        harness_pgid, harness_pid,
        "harness should be its own process group leader"
    );

    // The grandchild's PGID should equal the harness PID — this is the
    // invariant our orphan sweep fix relies on.
    let grandchild_pgid = unsafe { libc::getpgid(grandchild_pid) };
    assert_eq!(
        grandchild_pgid, harness_pid,
        "grandchild PGID should match harness PID (process group leader)"
    );

    // The grandchild's ppid is NOT the harness — it's the intermediate sh.
    // This proves the ppid-only check would miss it (the regression path).
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<super::BSDInfo>::zeroed();
        let ret = unsafe {
            super::proc_pidinfo(
                grandchild_pid,
                super::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr() as *mut libc::c_void,
                std::mem::size_of::<super::BSDInfo>() as libc::c_int,
            )
        };
        assert!(ret > 0, "proc_pidinfo should succeed for grandchild");
        let info = unsafe { info.assume_init() };
        assert_ne!(
            info.pbi_ppid as i32, harness_pid,
            "grandchild ppid must NOT be the harness (it's the intermediate sh)"
        );
    }

    // With skip_pids containing the harness PID, the grandchild's PGID
    // is in skip_pids — so it would NOT be flagged as an orphan.
    let skip_pids: Vec<u32> = vec![harness_pid as u32];
    assert!(
        skip_pids.contains(&(grandchild_pgid as u32)),
        "grandchild's PGID should be found in skip_pids"
    );

    // Cleanup: kill the process group.
    unsafe { libc::kill(-harness_pid, libc::SIGTERM) };
    let _ = harness.wait();
}
