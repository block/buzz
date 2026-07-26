use super::strip_baked_team_instructions_in_dir;
use crate::migration::test_support::{read_agents_json, write_agents_json};
use std::path::{Path, PathBuf};

const DELIMITER: &str = "\n\n---\n# Team Instructions\n";

fn base(dir: &Path) -> PathBuf {
    dir.join("agents")
}

fn agent_json(name: &str, prompt: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "pubkey": "",
        "relay_url": "ws://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "parallelism": 4,
        "system_prompt": prompt,
        "model": "gpt-x",
        "provider": "openai",
        "env_vars": { "API_KEY": "secret" },
        "start_on_app_launch": true,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "last_started_at": null,
        "last_stopped_at": null,
        "last_exit_code": null,
        "last_error": null
    })
}

fn prompt_of(dir: &Path, name: &str) -> Option<String> {
    read_agents_json(dir)
        .into_iter()
        .find(|r| r["name"] == name)
        .and_then(|r| r["system_prompt"].as_str().map(str::to_string))
}

/// Live `persona_content_hash` of the definition with `slug`, computed off the
/// store on disk through the same projection the drift badge uses.
fn definition_hash(dir: &Path, slug: &str) -> String {
    let records: Vec<crate::managed_agents::ManagedAgentRecord> =
        serde_json::from_value(serde_json::Value::Array(read_agents_json(dir))).unwrap();
    super::definition_hashes(&records)
        .remove(slug)
        .expect("definition present")
}

fn definition_json(slug: &str, prompt: &str) -> serde_json::Value {
    let mut record = agent_json(slug, Some(prompt));
    record["slug"] = serde_json::json!(slug);
    record["display_name"] = serde_json::json!("Paul");
    record
}

fn instance_json(slug: &str, prompt: &str, pinned: &str) -> serde_json::Value {
    let mut record = agent_json("Paul", Some(prompt));
    record["pubkey"] = serde_json::json!("p".repeat(64));
    record["persona_id"] = serde_json::json!(slug);
    record["persona_source_version"] = serde_json::json!(pinned);
    record
}

#[test]
fn strip_removes_the_baked_suffix_and_keeps_the_persona_body() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([agent_json(
            "Paul",
            Some(&format!("You are Paul.{DELIMITER}| Agent | Role |"))
        )]),
    );

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );
    assert_eq!(
        prompt_of(dir.path(), "Paul").as_deref(),
        Some("You are Paul.")
    );
}

#[test]
fn strip_splits_on_the_last_delimiter_occurrence() {
    // A persona body may quote a delimiter-shaped passage; only the final
    // occurrence is the boundary the removed compose_prompt() appended.
    let dir = tempfile::tempdir().unwrap();
    let quoted = format!("Header.{DELIMITER}quoted example");
    write_agents_json(
        dir.path(),
        &serde_json::json!([agent_json(
            "Duncan",
            Some(&format!("{quoted}{DELIMITER}live roster"))
        )]),
    );

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );
    assert_eq!(prompt_of(dir.path(), "Duncan").as_deref(), Some(&*quoted));
}

#[test]
fn strip_leaves_near_miss_lookalikes_untouched() {
    // Each variant differs from the producer boundary by exactly one detail:
    // no heading, heading on a different line, a single preceding newline,
    // and a trailing-space heading.
    let dir = tempfile::tempdir().unwrap();
    let lookalikes = [
        ("bare-rule", "Body.\n\n---\nMore body."),
        ("heading-inline", "Body.\n\n--- # Team Instructions\nMore."),
        ("single-newline", "Body.\n---\n# Team Instructions\nMore."),
        ("heading-only", "Body.\n\n# Team Instructions\nMore."),
        (
            "trailing-space",
            "Body.\n\n---\n# Team Instructions \nMore.",
        ),
    ];
    let records: Vec<serde_json::Value> = lookalikes
        .iter()
        .map(|(name, prompt)| agent_json(name, Some(prompt)))
        .collect();
    write_agents_json(dir.path(), &serde_json::Value::Array(records));
    let before = std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        0
    );
    let after = std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();
    assert_eq!(before, after, "no lookalike may be rewritten");
    for (name, prompt) in lookalikes {
        assert_eq!(prompt_of(dir.path(), name).as_deref(), Some(prompt));
    }
}

#[test]
fn strip_is_a_no_op_on_the_second_run() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([agent_json(
            "Paul",
            Some(&format!("You are Paul.{DELIMITER}roster"))
        )]),
    );
    let path = base(dir.path()).join("managed-agents.json");

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let after_first = std::fs::read_to_string(&path).unwrap();

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        0,
        "second run finds nothing to strip"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        after_first,
        "second run must not write"
    );
}

#[test]
fn strip_preserves_every_other_field() {
    let dir = tempfile::tempdir().unwrap();
    let mut record = agent_json("Thufir", Some(&format!("Prompt.{DELIMITER}roster")));
    record["pubkey"] = serde_json::json!("t".repeat(64));
    record["persona_id"] = serde_json::json!("sietch-tabr:thufir");
    record["team_id"] = serde_json::json!("team-uuid");
    record["persona_source_version"] = serde_json::json!("a".repeat(64));
    record["auth_tag"] = serde_json::json!("{\"tag\":\"value\"}");
    write_agents_json(dir.path(), &serde_json::json!([record]));

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let stored = read_agents_json(dir.path()).remove(0);
    assert_eq!(stored["system_prompt"], "Prompt.");
    assert_eq!(stored["pubkey"], "t".repeat(64));
    assert_eq!(stored["persona_id"], "sietch-tabr:thufir");
    assert_eq!(stored["team_id"], "team-uuid");
    assert_eq!(stored["persona_source_version"], "a".repeat(64));
    assert_eq!(stored["auth_tag"], "{\"tag\":\"value\"}");
    assert_eq!(stored["env_vars"]["API_KEY"], "secret");
    assert_eq!(stored["model"], "gpt-x");
    assert_eq!(stored["provider"], "openai");
    assert_eq!(stored["parallelism"], 4);
    assert_eq!(stored["created_at"], "2026-01-01T00:00:00Z");
}

#[test]
fn strip_writes_a_backup_once_and_never_clobbers_it() {
    let dir = tempfile::tempdir().unwrap();
    let original = format!("Alia prompt.{DELIMITER}roster");
    write_agents_json(
        dir.path(),
        &serde_json::json!([agent_json("Alia", Some(&original))]),
    );
    let bak = base(dir.path()).join("managed-agents.json.pre-team-suffix-strip.bak");

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );
    let bak_content = std::fs::read_to_string(&bak).unwrap();
    assert!(
        bak_content.contains("---\\n# Team Instructions"),
        "backup holds the pre-migration bytes"
    );

    // A later record acquiring the suffix must not replace the pristine backup.
    write_agents_json(
        dir.path(),
        &serde_json::json!([agent_json("Alia", Some(&format!("Edited.{DELIMITER}new")))]),
    );
    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(&bak).unwrap(),
        bak_content,
        "the first backup is never clobbered"
    );
}

#[test]
fn strip_nulls_a_prompt_that_was_only_the_baked_suffix() {
    // Head is empty: the record held nothing but the delimiter and the frozen
    // instructions. Storing Some("") would leave a phantom empty prompt that
    // reads as "the author wrote a blank prompt"; None is the absent-prompt
    // spelling the rest of the store uses.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([agent_json("Ghost", Some(&format!("{DELIMITER}roster")))]),
    );

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );
    assert!(
        read_agents_json(dir.path())[0]["system_prompt"].is_null(),
        "no phantom empty prompt"
    );
}

#[test]
fn strip_skips_records_without_a_prompt() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([agent_json("Promptless", None)]),
    );
    let before = std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap(),
        before
    );
}

#[test]
fn strip_on_a_missing_store_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base(dir.path())).unwrap();
    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        0
    );
}

#[test]
fn strip_on_an_unparseable_store_errors_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base(dir.path())).unwrap();
    let path = base(dir.path()).join("managed-agents.json");
    std::fs::write(&path, "{ not json").unwrap();

    let err = strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap_err();
    assert!(err.contains("failed to parse"), "unexpected error: {err}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{ not json",
        "a corrupt store is left for manual recovery"
    );
    assert!(
        !base(dir.path())
            .join("managed-agents.json.pre-team-suffix-strip.bak")
            .exists(),
        "no backup is taken when nothing can be migrated"
    );
}

#[test]
fn strip_repins_an_instance_that_was_current_before_the_strip() {
    // The definition's content hash changes when its prompt is stripped. An
    // instance pinned to the pre-strip hash was current, so it must stay
    // current — the user changed nothing and must not see a drift badge.
    let dir = tempfile::tempdir().unwrap();
    let baked = format!("You are Paul.{DELIMITER}frozen roster");
    write_agents_json(
        dir.path(),
        &serde_json::json!([definition_json("sietch-tabr:paul", &baked)]),
    );
    let pre_hash = definition_hash(dir.path(), "sietch-tabr:paul");
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition_json("sietch-tabr:paul", &baked),
            instance_json("sietch-tabr:paul", &baked, &pre_hash),
        ]),
    );

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        2
    );

    let post_hash = definition_hash(dir.path(), "sietch-tabr:paul");
    assert_ne!(pre_hash, post_hash, "stripping must change the drift basis");
    let instance = read_agents_json(dir.path())
        .into_iter()
        .find(|r| r["pubkey"].as_str() == Some(&"p".repeat(64)))
        .unwrap();
    assert_eq!(
        instance["persona_source_version"].as_str(),
        Some(post_hash.as_str()),
        "a previously-current instance must not start showing drift"
    );
}

#[test]
fn strip_leaves_an_already_drifted_instance_pin_alone() {
    // An instance pinned to something other than the definition's pre-strip
    // hash had genuinely drifted. Silently re-pinning it would erase a real
    // signal, so its stale pin — and its badge — survive.
    let dir = tempfile::tempdir().unwrap();
    let baked = format!("You are Paul.{DELIMITER}frozen roster");
    let stale = "d".repeat(64);
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition_json("sietch-tabr:paul", &baked),
            instance_json("sietch-tabr:paul", &baked, &stale),
        ]),
    );

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        2
    );

    let instance = read_agents_json(dir.path())
        .into_iter()
        .find(|r| r["pubkey"].as_str() == Some(&"p".repeat(64)))
        .unwrap();
    assert_eq!(
        instance["persona_source_version"].as_str(),
        Some(stale.as_str()),
        "real drift must keep its signal"
    );
}

#[cfg(unix)]
#[test]
fn strip_creates_the_backup_owner_only() {
    // The backup is a verbatim copy of a store that carries plaintext agent
    // nsecs whenever the keyring is unreachable, so it must be owner-only from
    // the initial open — a post-write chmod would leave a umask window in which
    // the secret is world-readable.
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let mut record = agent_json("Alia", Some(&format!("Alia prompt.{DELIMITER}roster")));
    record["private_key_nsec"] = serde_json::json!("nsec1exampleplaintextkey");
    write_agents_json(dir.path(), &serde_json::json!([record]));

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(dir.path())).unwrap(),
        1
    );

    let bak = base(dir.path()).join("managed-agents.json.pre-team-suffix-strip.bak");
    let mode = std::fs::metadata(&bak).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "backup of an inline nsec must be owner-only");
    assert!(
        std::fs::read_to_string(&bak)
            .unwrap()
            .contains("nsec1exampleplaintextkey"),
        "the fixture really did carry an inline key"
    );
}

#[cfg(unix)]
#[test]
fn strip_backs_up_beside_the_symlink_target() {
    // Dev worktrees symlink `agents/managed-agents.json` to a shared data dir
    // (`sync_shared_agent_data`). The sole recovery copy must land next to the
    // real data, not in whichever worktree booted first.
    let shared = tempfile::tempdir().unwrap();
    let worktree = tempfile::tempdir().unwrap();
    write_agents_json(
        shared.path(),
        &serde_json::json!([agent_json("Alia", Some(&format!("Alia.{DELIMITER}roster")))]),
    );
    std::fs::create_dir_all(base(worktree.path())).unwrap();
    std::os::unix::fs::symlink(
        base(shared.path()).join("managed-agents.json"),
        base(worktree.path()).join("managed-agents.json"),
    )
    .unwrap();

    assert_eq!(
        strip_baked_team_instructions_in_dir(&base(worktree.path())).unwrap(),
        1
    );

    assert!(
        base(shared.path())
            .join("managed-agents.json.pre-team-suffix-strip.bak")
            .exists(),
        "backup belongs beside the shared store"
    );
    assert!(
        !base(worktree.path())
            .join("managed-agents.json.pre-team-suffix-strip.bak")
            .exists(),
        "no orphan backup in the worktree data dir"
    );
    assert_eq!(prompt_of(shared.path(), "Alia").as_deref(), Some("Alia."));
}
