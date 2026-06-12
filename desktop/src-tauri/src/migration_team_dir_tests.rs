use super::*;
use std::path::Path;

fn write_agents_json(dir: &Path, records: &serde_json::Value) {
    std::fs::create_dir_all(dir.join("agents")).unwrap();
    std::fs::write(
        dir.join("agents/managed-agents.json"),
        serde_json::to_vec_pretty(records).unwrap(),
    )
    .unwrap();
}

fn read_agents_json(dir: &Path) -> Vec<serde_json::Value> {
    let content = std::fs::read_to_string(dir.join("agents/managed-agents.json")).unwrap();
    serde_json::from_str(&content).unwrap()
}

// ── reconcile_team_dirs_in_file tests ────────────────────────────────

#[test]
fn team_dir_reconcile_rewrites_worktree_path() {
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents")).unwrap();
    // Team must exist on disk for the reconcile to proceed past the existence gate.
    std::fs::create_dir_all(canonical.join("agents/teams/com.wpfleger.sietch-tabr")).unwrap();

    let worktree_pack_path = format!(
        "{}/agents/packs/com.wpfleger.sietch-tabr",
        parent
            .path()
            .join("xyz.block.buzz.app.dev.worktree-my-branch")
            .display()
    );
    let expected_path = format!(
        "{}/agents/teams/com.wpfleger.sietch-tabr",
        canonical.display()
    );

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Paul",
            "persona_pack_path": worktree_pack_path
        }]),
    );

    reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);

    let records = read_agents_json(&canonical);
    assert_eq!(records[0]["persona_team_dir"], expected_path);
    // Old field name should be removed
    assert!(records[0].get("persona_pack_path").is_none());
}

#[test]
fn team_dir_reconcile_rewrites_new_field_name() {
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents")).unwrap();
    // Team must exist on disk for the reconcile to proceed past the existence gate.
    std::fs::create_dir_all(canonical.join("agents/teams/com.wpfleger.sietch-tabr")).unwrap();

    let worktree_team_path = format!(
        "{}/agents/teams/com.wpfleger.sietch-tabr",
        parent
            .path()
            .join("xyz.block.buzz.app.dev.worktree-my-branch")
            .display()
    );
    let expected_path = format!(
        "{}/agents/teams/com.wpfleger.sietch-tabr",
        canonical.display()
    );

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Paul",
            "persona_team_dir": worktree_team_path
        }]),
    );

    reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);

    let records = read_agents_json(&canonical);
    assert_eq!(records[0]["persona_team_dir"], expected_path);
}

#[test]
fn team_dir_reconcile_leaves_canonical_path_unchanged() {
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents")).unwrap();

    let canonical_path = format!(
        "{}/agents/teams/com.wpfleger.sietch-tabr",
        canonical.display()
    );

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Duncan",
            "persona_team_dir": canonical_path
        }]),
    );

    let before = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();
    reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);
    let after = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();

    assert_eq!(before, after);
}

#[test]
fn team_dir_reconcile_skips_records_without_team_dir() {
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents")).unwrap();

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Test Agent",
            "agent_command": "buzz-agent"
        }]),
    );

    let before = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();
    reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);
    let after = std::fs::read_to_string(canonical.join("agents/managed-agents.json")).unwrap();

    assert_eq!(before, after);
}

#[test]
fn team_dir_reconcile_is_idempotent() {
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents")).unwrap();
    // Team must exist on disk for the reconcile to proceed past the existence gate.
    std::fs::create_dir_all(canonical.join("agents/teams/com.wpfleger.sietch-tabr")).unwrap();

    let worktree_pack_path = format!(
        "{}/agents/packs/com.wpfleger.sietch-tabr",
        parent
            .path()
            .join("xyz.block.buzz.app.dev.worktree-my-branch")
            .display()
    );

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Paul",
            "persona_pack_path": worktree_pack_path
        }]),
    );

    let path = canonical.join("agents/managed-agents.json");
    reconcile_team_dirs_in_file(&path, &canonical);
    let after_first = std::fs::read_to_string(&path).unwrap();
    reconcile_team_dirs_in_file(&path, &canonical);
    let after_second = std::fs::read_to_string(&path).unwrap();

    assert_eq!(after_first, after_second);
}

#[test]
fn team_dir_reconcile_heals_legacy_release_path() {
    // Record stored under the old bundle id and old packs segment should be
    // rewritten to the new bundle id and teams segment.
    let parent = tempfile::tempdir().unwrap();
    let release_dir = parent.path().join("xyz.block.buzz.app");
    std::fs::create_dir_all(release_dir.join("agents/teams/com.example.team")).unwrap();

    let legacy_path = format!(
        "{}/agents/packs/com.example.team",
        parent.path().join("xyz.block.sprout.app").display()
    );
    let expected_path = format!("{}/agents/teams/com.example.team", release_dir.display());

    write_agents_json(
        &release_dir,
        &serde_json::json!([{
            "name": "Stilgar",
            "persona_team_dir": legacy_path
        }]),
    );

    reconcile_team_dirs_in_file(
        &release_dir.join("agents/managed-agents.json"),
        &release_dir,
    );

    let records = read_agents_json(&release_dir);
    assert_eq!(records[0]["persona_team_dir"], expected_path);
}

#[test]
fn team_dir_reconcile_leaves_record_when_team_missing() {
    // When the expected target does not exist, the record must be left untouched
    // and the file must not be rewritten at all (no churn).
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents")).unwrap();
    // Intentionally do NOT create agents/teams/com.example.missing.

    let stale_path = format!(
        "{}/agents/packs/com.example.missing",
        parent
            .path()
            .join("xyz.block.buzz.app.dev.worktree-old")
            .display()
    );

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Jessica",
            "persona_pack_path": stale_path
        }]),
    );

    let path = canonical.join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();
    reconcile_team_dirs_in_file(&path, &canonical);
    let after = std::fs::read_to_string(&path).unwrap();

    // File must be byte-identical — no churn.
    assert_eq!(before, after);
    // Record still has the old field and value.
    let records = read_agents_json(&canonical);
    assert_eq!(records[0]["persona_pack_path"], stale_path);
    assert!(records[0].get("persona_team_dir").is_none());
}

#[test]
fn team_dir_reconcile_heals_to_symlinked_team_dir() {
    // When agents/teams/<id> is a symlink to a real directory elsewhere,
    // Path::exists follows the symlink, so the rewrite should proceed.
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents")).unwrap();

    // Real team lives outside the canonical dir; a symlink points to it.
    let real_team = parent.path().join("real-team-store/com.example.team");
    std::fs::create_dir_all(&real_team).unwrap();
    let symlink_target = canonical.join("agents/teams/com.example.team");
    std::fs::create_dir_all(canonical.join("agents/teams")).unwrap();
    std::os::unix::fs::symlink(&real_team, &symlink_target).unwrap();

    let stale_path = format!(
        "{}/agents/packs/com.example.team",
        parent
            .path()
            .join("xyz.block.buzz.app.dev.worktree-old")
            .display()
    );
    let expected_path = format!("{}/agents/teams/com.example.team", canonical.display());

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Chani",
            "persona_pack_path": stale_path
        }]),
    );

    reconcile_team_dirs_in_file(&canonical.join("agents/managed-agents.json"), &canonical);

    let records = read_agents_json(&canonical);
    assert_eq!(records[0]["persona_team_dir"], expected_path);
}

#[test]
fn team_dir_reconcile_skips_dangling_candidate_symlink() {
    // When agents/teams/<id> is a symlink whose target does not exist,
    // Path::exists returns false, so the record must be left unchanged.
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(canonical.join("agents/teams")).unwrap();

    // Create a dangling symlink at the expected location.
    let dangling_target = parent.path().join("nonexistent-dir");
    let symlink_path = canonical.join("agents/teams/com.example.gone");
    std::os::unix::fs::symlink(&dangling_target, &symlink_path).unwrap();

    let stale_path = format!(
        "{}/agents/packs/com.example.gone",
        parent
            .path()
            .join("xyz.block.buzz.app.dev.worktree-old")
            .display()
    );

    write_agents_json(
        &canonical,
        &serde_json::json!([{
            "name": "Gurney",
            "persona_pack_path": stale_path
        }]),
    );

    let path = canonical.join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();
    reconcile_team_dirs_in_file(&path, &canonical);
    let after = std::fs::read_to_string(&path).unwrap();

    assert_eq!(before, after);
    let records = read_agents_json(&canonical);
    assert_eq!(records[0]["persona_pack_path"], stale_path);
}

// ── reconcile_target_dir tests ───────────────────────────────────────

#[test]
fn reconcile_target_dir_release_with_existing_dev_sibling_returns_self() {
    // Release build must reconcile its OWN dir, even when a dev sibling exists.
    let parent = tempfile::tempdir().unwrap();
    let release_dir = parent.path().join("xyz.block.buzz.app");
    let dev_dir = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(&release_dir).unwrap();
    std::fs::create_dir_all(&dev_dir).unwrap();

    assert_eq!(reconcile_target_dir(release_dir.clone()), release_dir);
}

#[test]
fn reconcile_target_dir_worktree_dev_with_canonical_sibling_returns_canonical() {
    // A worktree dev instance defers to the canonical dev dir when it exists.
    let parent = tempfile::tempdir().unwrap();
    let worktree_dir = parent.path().join("xyz.block.buzz.app.dev.mybranch");
    let canonical_dir = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(&worktree_dir).unwrap();
    std::fs::create_dir_all(&canonical_dir).unwrap();

    assert_eq!(reconcile_target_dir(worktree_dir), canonical_dir);
}

#[test]
fn reconcile_target_dir_canonical_dev_returns_self() {
    // The canonical dev dir is itself a dev instance; it should return itself
    // (canonical_dev_data_dir points at the same path, and that path exists).
    let parent = tempfile::tempdir().unwrap();
    let canonical_dir = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    std::fs::create_dir_all(&canonical_dir).unwrap();

    assert_eq!(reconcile_target_dir(canonical_dir.clone()), canonical_dir);
}

#[test]
fn reconcile_target_dir_release_without_dev_sibling_returns_self() {
    // Release build with no dev sibling present — stays on its own dir.
    let parent = tempfile::tempdir().unwrap();
    let release_dir = parent.path().join("xyz.block.buzz.app");
    std::fs::create_dir_all(&release_dir).unwrap();

    assert_eq!(reconcile_target_dir(release_dir.clone()), release_dir);
}

#[test]
fn reconcile_target_dir_worktree_dev_without_canonical_sibling_returns_self() {
    // Worktree dev instance with no canonical sibling present — stays on itself.
    let parent = tempfile::tempdir().unwrap();
    let worktree_dir = parent.path().join("xyz.block.buzz.app.dev.mybranch");
    std::fs::create_dir_all(&worktree_dir).unwrap();

    assert_eq!(reconcile_target_dir(worktree_dir.clone()), worktree_dir);
}
