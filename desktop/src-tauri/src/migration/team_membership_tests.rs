use super::repair_team_membership_in_dir;
use crate::migration::test_support::{
    read_agents_json, read_teams_json, write_agents_json, write_teams_json,
};
use std::path::{Path, PathBuf};

fn base(dir: &Path) -> PathBuf {
    dir.join("agents")
}

/// A key-less definition record: `pubkey == ""`, persona id == `slug`.
/// `source_team` is the manifest id; `source_team_persona_slug` is the
/// pre-namespacing bare slug a stale team id would carry.
fn definition(slug: &str, source_team: &str, bare_slug: &str) -> serde_json::Value {
    serde_json::json!({
        "name": slug,
        "pubkey": "",
        "relay_url": "ws://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "parallelism": 4,
        "system_prompt": "prompt",
        "model": "gpt-x",
        "provider": "openai",
        "env_vars": {},
        "start_on_app_launch": true,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "slug": slug,
        "source_team": source_team,
        "source_team_persona_slug": bare_slug,
    })
}

/// A standalone definition with no team provenance (persona id == slug).
fn standalone_definition(slug: &str) -> serde_json::Value {
    serde_json::json!({
        "name": slug,
        "pubkey": "",
        "relay_url": "ws://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "parallelism": 4,
        "system_prompt": "prompt",
        "model": "gpt-x",
        "provider": "openai",
        "env_vars": {},
        "start_on_app_launch": true,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "slug": slug,
    })
}

/// A running instance record: `pubkey` set, linked to a persona by `persona_id`.
fn instance(pubkey_seed: char, persona_id: &str, team_id: Option<&str>) -> serde_json::Value {
    let mut record = serde_json::json!({
        "name": persona_id,
        "pubkey": pubkey_seed.to_string().repeat(64),
        "relay_url": "ws://localhost:3000",
        "acp_command": "buzz-acp",
        "agent_command": "goose",
        "agent_args": [],
        "mcp_command": "",
        "turn_timeout_seconds": 320,
        "parallelism": 4,
        "system_prompt": "prompt",
        "model": "gpt-x",
        "provider": "openai",
        "env_vars": {},
        "start_on_app_launch": true,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
        "persona_id": persona_id,
    });
    record["team_id"] = match team_id {
        Some(id) => serde_json::json!(id),
        None => serde_json::Value::Null,
    };
    record
}

fn team(id: &str, persona_ids: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": "Sietch Tabr",
        "description": null,
        "persona_ids": persona_ids,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    })
}

fn team_persona_ids(dir: &Path, id: &str) -> Vec<String> {
    read_teams_json(dir)
        .into_iter()
        .find(|t| t["id"] == id)
        .unwrap()["persona_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

fn instance_team_id(dir: &Path, pubkey_seed: char) -> Option<String> {
    read_agents_json(dir)
        .into_iter()
        .find(|r| r["pubkey"].as_str() == Some(&pubkey_seed.to_string().repeat(64)))
        .unwrap()["team_id"]
        .as_str()
        .map(str::to_string)
}

const TEAM_ID: &str = "ab5c038c-1b12-46e2-8283-d6f7c0606fce";
const ST: &str = "com.wpfleger.sietch-tabr";

/// Will's pre-fix store: the team holds four bare pre-namespacing ids plus one
/// resolvable standalone id. Each bare id names exactly one team persona, so
/// all four are rewritten to their namespaced slug and the standalone id is
/// left untouched — the class the save path silently drops.
#[test]
fn rewrites_bare_ids_to_namespaced_slugs() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(
        dir.path(),
        &serde_json::json!([team(
            TEAM_ID,
            &["369695d6", "thufir", "paul", "duncan", "alia"]
        )]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            standalone_definition("369695d6"),
            definition("sietch-tabr:thufir", ST, "thufir"),
            definition("sietch-tabr:paul", ST, "paul"),
            definition("sietch-tabr:duncan", ST, "duncan"),
            definition("sietch-tabr:alia", ST, "alia"),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 4);
    assert_eq!(
        team_persona_ids(dir.path(), TEAM_ID),
        vec![
            "369695d6",
            "sietch-tabr:thufir",
            "sietch-tabr:paul",
            "sietch-tabr:duncan",
            "sietch-tabr:alia",
        ]
    );
}

/// A directory-backed team scopes candidates by its `source_dir` name (the pack
/// manifest id), so a bare slug that appears under two different source teams is
/// disambiguated to the one this team is sourced from.
#[test]
fn scopes_candidates_by_source_dir_for_directory_backed_team() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = team(TEAM_ID, &["thufir"]);
    t["source_dir"] = serde_json::json!(format!("/packs/{ST}"));
    write_teams_json(dir.path(), &serde_json::json!([t]));
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:thufir", ST, "thufir"),
            // A collision: a different team also has a persona whose bare slug
            // is "thufir". Without source scoping this would be ambiguous.
            definition("other:thufir", "com.other.pack", "thufir"),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    assert_eq!(
        team_persona_ids(dir.path(), TEAM_ID),
        vec!["sietch-tabr:thufir"]
    );
}

/// A bare id that names two personas with no usable scope is ambiguous: the
/// migration leaves it in place (strictly safer than the save path, which drops
/// it) and the file is not rewritten.
#[test]
fn leaves_ambiguous_id_in_place_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    // Detached team (no source_dir) with a single stale member => no resolvable
    // sibling to infer a source-team scope from.
    write_teams_json(dir.path(), &serde_json::json!([team(TEAM_ID, &["thufir"])]));
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:thufir", ST, "thufir"),
            definition("other:thufir", "com.other.pack", "thufir"),
        ]),
    );
    let before = std::fs::read_to_string(base(dir.path()).join("teams.json")).unwrap();

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 0);
    assert_eq!(team_persona_ids(dir.path(), TEAM_ID), vec!["thufir"]);
    assert_eq!(
        std::fs::read_to_string(base(dir.path()).join("teams.json")).unwrap(),
        before,
        "an ambiguous-only store is never rewritten"
    );
    assert!(
        !base(dir.path())
            .join("teams.json.pre-team-membership-repair.bak")
            .exists(),
        "no backup when nothing is repaired"
    );
}

/// A detached team infers its source-team scope from the unique `source_team`
/// among its already-resolvable members, so a bare id is disambiguated even
/// without a `source_dir`.
#[test]
fn infers_scope_from_resolvable_siblings_when_detached() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(
        dir.path(),
        &serde_json::json!([team(TEAM_ID, &["sietch-tabr:paul", "thufir"])]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:paul", ST, "paul"),
            definition("sietch-tabr:thufir", ST, "thufir"),
            definition("other:thufir", "com.other.pack", "thufir"),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    assert_eq!(
        team_persona_ids(dir.path(), TEAM_ID),
        vec!["sietch-tabr:paul", "sietch-tabr:thufir"]
    );
}

/// Backfill sets `team_id` on an instance whose persona is a team member but
/// whose own `team_id` is null (the Gurney case), and leaves an already-bound
/// instance untouched (a persona shared across teams keeps its binding).
#[test]
fn backfills_null_team_id_but_never_re_points_a_bound_instance() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(
        dir.path(),
        &serde_json::json!([team(TEAM_ID, &["sietch-tabr:gurney", "sietch-tabr:hayt"])]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:gurney", ST, "gurney"),
            definition("sietch-tabr:hayt", ST, "hayt"),
            instance('g', "sietch-tabr:gurney", None),
            instance('h', "sietch-tabr:hayt", Some("other-team")),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    assert_eq!(instance_team_id(dir.path(), 'g').as_deref(), Some(TEAM_ID));
    assert_eq!(
        instance_team_id(dir.path(), 'h').as_deref(),
        Some("other-team"),
        "an already-bound instance is never re-pointed"
    );
}

/// A legacy unbound instance whose persona belongs to *two* teams is left
/// unbound: JSON team order is not ownership evidence, and the product permits
/// one persona under multiple teams with distinct instructions. Its team
/// sibling — a persona in only one team — is still backfilled in the same pass.
#[test]
fn leaves_unbound_instance_of_a_multi_team_persona_unbound() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(
        dir.path(),
        &serde_json::json!([
            team(TEAM_ID, &["sietch-tabr:duncan", "sietch-tabr:paul"]),
            team("other-team", &["sietch-tabr:duncan"]),
        ]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:duncan", ST, "duncan"),
            definition("sietch-tabr:paul", ST, "paul"),
            instance('d', "sietch-tabr:duncan", None),
            instance('p', "sietch-tabr:paul", None),
        ]),
    );

    // Only Paul (single-team) is backfilled; Duncan (two teams) stays unbound.
    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    assert_eq!(instance_team_id(dir.path(), 'd'), None);
    assert_eq!(instance_team_id(dir.path(), 'p').as_deref(), Some(TEAM_ID));
}

/// A persona listed twice within a *single* team is not ambiguity — the storage
/// boundary does not dedupe `persona_ids`. Its unbound instance is still bound
/// to that one team; only a *distinct* second team poisons the entry.
#[test]
fn same_team_duplicate_persona_id_still_backfills() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(
        dir.path(),
        &serde_json::json!([team(TEAM_ID, &["sietch-tabr:duncan", "sietch-tabr:duncan"])]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:duncan", ST, "duncan"),
            instance('d', "sietch-tabr:duncan", None),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    assert_eq!(instance_team_id(dir.path(), 'd').as_deref(), Some(TEAM_ID));
}

/// A stale binding — the bound team no longer lists the instance's persona (a
/// "keep agents" removal left it behind) — is cleared when no other single team
/// claims the persona, so the kept instance stops drawing that team's
/// instructions at spawn.
#[test]
fn clears_stale_binding_when_persona_left_its_team() {
    let dir = tempfile::tempdir().unwrap();
    // The team no longer lists gurney; the instance is still bound to it.
    write_teams_json(
        dir.path(),
        &serde_json::json!([team(TEAM_ID, &["sietch-tabr:paul"])]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:gurney", ST, "gurney"),
            definition("sietch-tabr:paul", ST, "paul"),
            instance('g', "sietch-tabr:gurney", Some(TEAM_ID)),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    assert_eq!(instance_team_id(dir.path(), 'g'), None);
}

/// A stale binding is *re-pointed* — not merely cleared — when the persona now
/// belongs to exactly one other team, matching the single-evidence backfill
/// rule.
#[test]
fn repoints_stale_binding_to_the_sole_successor_team() {
    let dir = tempfile::tempdir().unwrap();
    // gurney left TEAM_ID but is the sole member of other-team.
    write_teams_json(
        dir.path(),
        &serde_json::json!([
            team(TEAM_ID, &["sietch-tabr:paul"]),
            team("other-team", &["sietch-tabr:gurney"]),
        ]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:gurney", ST, "gurney"),
            definition("sietch-tabr:paul", ST, "paul"),
            instance('g', "sietch-tabr:gurney", Some(TEAM_ID)),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    assert_eq!(
        instance_team_id(dir.path(), 'g').as_deref(),
        Some("other-team")
    );
}

/// A binding whose team still lists the persona is authoritative — a repair pass
/// leaves it untouched even when that persona also belongs to another team.
#[test]
fn leaves_live_binding_untouched_for_multi_team_persona() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(
        dir.path(),
        &serde_json::json!([
            team(TEAM_ID, &["sietch-tabr:duncan"]),
            team("other-team", &["sietch-tabr:duncan"]),
        ]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:duncan", ST, "duncan"),
            instance('d', "sietch-tabr:duncan", Some(TEAM_ID)),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 0);
    assert_eq!(instance_team_id(dir.path(), 'd').as_deref(), Some(TEAM_ID));
}

/// A store that needs no repair is a clean no-op: `Ok(0)`, no write, no backup.
#[test]
fn clean_store_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(
        dir.path(),
        &serde_json::json!([team(TEAM_ID, &["sietch-tabr:paul"])]),
    );
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:paul", ST, "paul"),
            instance('p', "sietch-tabr:paul", Some(TEAM_ID)),
        ]),
    );
    let teams_before = std::fs::read_to_string(base(dir.path()).join("teams.json")).unwrap();
    let agents_before =
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 0);
    assert_eq!(
        std::fs::read_to_string(base(dir.path()).join("teams.json")).unwrap(),
        teams_before
    );
    assert_eq!(
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap(),
        agents_before
    );
}

/// The full repair is idempotent: a second boot over the already-repaired store
/// finds nothing to do and does not write.
#[test]
fn second_run_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(dir.path(), &serde_json::json!([team(TEAM_ID, &["thufir"])]));
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:thufir", ST, "thufir"),
            instance('t', "sietch-tabr:thufir", None),
        ]),
    );

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 2);
    let teams_after = std::fs::read_to_string(base(dir.path()).join("teams.json")).unwrap();
    let agents_after =
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap();

    assert_eq!(
        repair_team_membership_in_dir(&base(dir.path())).unwrap(),
        0,
        "second run finds nothing"
    );
    assert_eq!(
        std::fs::read_to_string(base(dir.path()).join("teams.json")).unwrap(),
        teams_after
    );
    assert_eq!(
        std::fs::read_to_string(base(dir.path()).join("managed-agents.json")).unwrap(),
        agents_after
    );
}

#[test]
fn missing_store_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base(dir.path())).unwrap();
    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 0);
}

#[test]
fn unparseable_store_errors_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(base(dir.path())).unwrap();
    let teams_path = base(dir.path()).join("teams.json");
    std::fs::write(&teams_path, "{ not json").unwrap();
    write_agents_json(dir.path(), &serde_json::json!([]));

    let err = repair_team_membership_in_dir(&base(dir.path())).unwrap_err();
    assert!(err.contains("failed to parse"), "unexpected error: {err}");
    assert_eq!(
        std::fs::read_to_string(&teams_path).unwrap(),
        "{ not json",
        "a corrupt store is left for manual recovery"
    );
}

/// The teams.json backup captures the pre-migration bytes and is written once.
#[test]
fn writes_teams_backup_once() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(dir.path(), &serde_json::json!([team(TEAM_ID, &["thufir"])]));
    write_agents_json(
        dir.path(),
        &serde_json::json!([definition("sietch-tabr:thufir", ST, "thufir")]),
    );
    let bak = base(dir.path()).join("teams.json.pre-team-membership-repair.bak");

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 1);
    let bak_content = std::fs::read_to_string(&bak).unwrap();
    assert!(
        bak_content.contains("\"thufir\""),
        "backup holds the pre-migration stale id"
    );
}

/// Both pristine backups are created BEFORE either live store is rewritten, so
/// a crash between the two writes still leaves a full recovery pair (Carl's
/// backup-contract finding). A stale bare slug on the team (drives the teams
/// rewrite) plus an unbound instance (drives the agents backfill) exercises
/// both stores; each backup must hold the pre-migration bytes.
#[test]
fn both_backups_precede_either_live_write() {
    let dir = tempfile::tempdir().unwrap();
    write_teams_json(dir.path(), &serde_json::json!([team(TEAM_ID, &["thufir"])]));
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            definition("sietch-tabr:thufir", ST, "thufir"),
            instance('t', "sietch-tabr:thufir", None),
        ]),
    );
    let teams_bak = base(dir.path()).join("teams.json.pre-team-membership-repair.bak");
    let agents_bak = base(dir.path()).join("managed-agents.json.pre-team-membership-repair.bak");

    assert_eq!(repair_team_membership_in_dir(&base(dir.path())).unwrap(), 2);

    // teams.json backup holds the stale bare slug (pre-rewrite bytes).
    let teams_bak_content = std::fs::read_to_string(&teams_bak).unwrap();
    assert!(
        teams_bak_content.contains("\"thufir\"")
            && !teams_bak_content.contains("sietch-tabr:thufir"),
        "teams backup captures pre-rewrite bytes"
    );
    // managed-agents.json backup holds the null binding (pre-backfill bytes).
    let agents_bak_content = std::fs::read_to_string(&agents_bak).unwrap();
    assert!(
        agents_bak_content.contains("\"team_id\": null"),
        "agents backup captures pre-backfill bytes"
    );
}

// ── repair→detach orchestration gate (Carl's finding #2) ──────────────────

use super::orchestrate_repair_then_detach;
use std::cell::Cell;

/// A failed repair must SKIP the directory-backed detach: detach clears
/// `source_dir`, the disambiguating evidence a clean-repair retry needs, so
/// running it after a repair error would let the original membership-loss path
/// recur on the next boot.
#[test]
fn failed_repair_skips_detach() {
    let detach_ran = Cell::new(false);
    orchestrate_repair_then_detach(
        || Err("repair write failed".to_string()),
        || {
            detach_ran.set(true);
            Ok(0)
        },
    );
    assert!(
        !detach_ran.get(),
        "detach must not run when repair failed — source_dir is preserved for retry"
    );
}

/// A successful repair runs detach, whether or not the repair changed anything
/// (a clean-store boot with directory-backed teams still needs detaching).
#[test]
fn successful_repair_runs_detach() {
    let detach_ran = Cell::new(false);
    orchestrate_repair_then_detach(
        || Ok(0),
        || {
            detach_ran.set(true);
            Ok(1)
        },
    );
    assert!(
        detach_ran.get(),
        "detach runs after a clean repair even when repair changed nothing"
    );
}

/// End-to-end discriminating proof: a failed repair must leave a
/// directory-backed team's `source_dir` intact, because the gate skips the real
/// detach op that would otherwise clear it. The store here is fully valid — so
/// detach WOULD succeed and strip `source_dir` if the gate let it run — which
/// is what makes this catch a gate that runs detach unconditionally.
#[test]
fn failed_repair_preserves_source_dir_against_real_detach() {
    let dir = tempfile::tempdir().unwrap();
    let base_dir = base(dir.path());
    let mut t = team(TEAM_ID, &["sietch-tabr:thufir"]);
    t["source_dir"] = serde_json::json!(format!("/packs/{ST}"));
    write_teams_json(dir.path(), &serde_json::json!([t]));
    write_agents_json(
        dir.path(),
        &serde_json::json!([definition("sietch-tabr:thufir", ST, "thufir")]),
    );

    orchestrate_repair_then_detach(
        || Err("repair backup write failed".to_string()),
        || super::super::detach::detach_directory_backed_teams_in_dir(&base_dir),
    );

    let source_dir = read_teams_json(dir.path())
        .into_iter()
        .find(|t| t["id"] == TEAM_ID)
        .unwrap()["source_dir"]
        .clone();
    assert_eq!(
        source_dir,
        serde_json::json!(format!("/packs/{ST}")),
        "a failed repair must preserve source_dir — detach never ran to clear it"
    );
}
