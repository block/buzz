use super::*;
use std::collections::BTreeMap;

/// Build a minimal AgentDefinition with the fields that `rewrite_persona_md` reads.
fn persona(
    display_name: &str,
    runtime: Option<&str>,
    avatar_url: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
) -> AgentDefinition {
    // system_prompt matches the SAMPLE_MD body so the "no prompt edit" path
    // is taken in rewrite_persona_md (body preserved byte-for-byte).
    persona_with_prompt(
        display_name,
        runtime,
        avatar_url,
        provider,
        model,
        "You are Paul.\n",
    )
}

/// Like `persona` but with an explicit system_prompt value.
fn persona_with_prompt(
    display_name: &str,
    runtime: Option<&str>,
    avatar_url: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
    system_prompt: &str,
) -> AgentDefinition {
    AgentDefinition {
        id: "test-id".to_string(),
        display_name: display_name.to_string(),
        avatar_url: avatar_url.map(str::to_string),
        system_prompt: system_prompt.to_string(),
        runtime: runtime.map(str::to_string),
        model: model.map(str::to_string),
        provider: provider.map(str::to_string),
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: Some("team-1".to_string()),
        source_team_persona_slug: Some("paul".to_string()),
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        mcp_toolsets: None,
        parallelism: None,
        created_at: "2025-01-01T00:00:00Z".to_string(),
        updated_at: "2025-01-01T00:00:00Z".to_string(),
    }
}

const SAMPLE_MD: &str = "\
---
name: paul
display_name: \"Paul\"
description: \"An orchestrator.\"
model: goose-claude-4-6-opus
runtime: goose
extra_key: keep-me
---
You are Paul.
";

// ── rewrite_persona_md unit tests ─────────────────────────────────────────

#[test]
fn test_rewrite_model_provider_joined_and_body_preserved() {
    let p = persona(
        "Paul",
        Some("goose"),
        None,
        Some("databricks_v2"),
        Some("goose-claude-opus-4-8"),
    );
    let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

    // Body is byte-preserved.
    assert!(
        result.ends_with("\nYou are Paul.\n"),
        "body not preserved: {result:?}"
    );

    // model key is the joined form.
    assert!(
        result.contains("model: databricks_v2:goose-claude-opus-4-8"),
        "joined model missing: {result}"
    );

    // No separate provider key.
    assert!(
        !result.contains("provider:"),
        "separate provider key must not be emitted: {result}"
    );

    // Unrelated key preserved.
    assert!(
        result.contains("extra_key: keep-me"),
        "extra key lost: {result}"
    );

    // Still valid frontmatter (parses cleanly).
    assert!(result.starts_with("---\n"), "must start with ---");
}

#[test]
fn test_rewrite_bare_model_when_provider_none() {
    let p = persona("Paul", Some("goose"), None, None, Some("bare-model-id"));
    let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

    assert!(
        result.contains("model: bare-model-id"),
        "bare model missing: {result}"
    );
    assert!(
        !result.contains("provider:"),
        "provider key must not be emitted: {result}"
    );
}

#[test]
fn test_rewrite_runtime_removed_when_none() {
    let p = persona("Paul", None, None, None, Some("some-model"));
    let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

    // runtime was in source but persona.runtime is None — key must be removed.
    assert!(
        !result.contains("runtime:"),
        "runtime key should be removed when None: {result}"
    );
}

#[test]
fn test_rewrite_preserves_description_and_name_and_extra_keys() {
    let p = persona(
        "Paul Updated",
        Some("goose"),
        None,
        Some("anthropic"),
        Some("claude-opus-4"),
    );
    let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

    // name and description are not persona record fields — must survive untouched.
    assert!(result.contains("name: paul"), "name key lost: {result}");
    assert!(
        result.contains("description:"),
        "description key lost: {result}"
    );
    assert!(
        result.contains("extra_key: keep-me"),
        "extra_key lost: {result}"
    );

    // display_name updated.
    assert!(
        result.contains("display_name: Paul Updated")
            || result.contains("display_name: \"Paul Updated\""),
        "display_name not updated: {result}"
    );
}

#[test]
fn test_rewrite_no_provider_no_model_removes_model_key() {
    let p = persona("Paul", None, None, None, None);
    let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

    // When both provider and model are cleared, the model key is removed.
    assert!(
        !result.contains("model:"),
        "model key should be removed when both absent: {result}"
    );
}

#[test]
fn test_rewrite_avatar_set_and_cleared() {
    let with_avatar = persona(
        "Paul",
        Some("goose"),
        Some("data:image/png;base64,abc"),
        Some("openai"),
        Some("gpt-4o"),
    );
    let result = rewrite_persona_md(SAMPLE_MD, &with_avatar, "You are Paul.\n", None).unwrap();
    assert!(
        result.contains("avatar:"),
        "avatar key should be set: {result}"
    );

    let without_avatar = persona("Paul", Some("goose"), None, Some("openai"), Some("gpt-4o"));
    let result = rewrite_persona_md(SAMPLE_MD, &without_avatar, "You are Paul.\n", None).unwrap();
    assert!(
        !result.contains("avatar:"),
        "avatar key should be absent when None: {result}"
    );
}

#[test]
fn test_rewrite_body_not_replaced_by_system_prompt() {
    // system_prompt on the AgentDefinition is the COMPOSED prompt (body + pack instructions).
    // The body of the .persona.md must not be replaced with it.
    let p = persona(
        "Paul",
        Some("goose"),
        None,
        Some("databricks_v2"),
        Some("goose-claude-opus-4-8"),
    );
    let result = rewrite_persona_md(SAMPLE_MD, &p, "You are Paul.\n", None).unwrap();

    // The raw body from the file ("You are Paul.") is preserved.
    assert!(
        result.ends_with("You are Paul.\n"),
        "raw body must be preserved, not replaced by composed system_prompt: {result:?}"
    );
    // The composed instructions suffix must NOT appear in the file body.
    assert!(
        !result.contains("# Team Instructions"),
        "composed system_prompt must not be written to body: {result}"
    );
}

// ── body (system_prompt) write-back tests ─────────────────────────────────
//
// These tests exercise the compose_prompt inversion logic in rewrite_persona_md.

/// Frontmatter-only MD (no body to preserve) for prompt tests.
const PROMPT_MD: &str = "\
---
name: paul
display_name: \"Paul\"
model: goose-claude-4-6-opus
---
You are Paul.
";

#[test]
fn test_prompt_edited_with_pack_instructions_body_rewritten() {
    // User edits the prompt. system_prompt = new_raw_body + separator + instructions.
    // Body in file should be updated to new_raw_body; Team Instructions must NOT appear.
    let instructions = "Follow the rules.";
    let new_raw_body = "You are Paul, a wise orchestrator.";
    let composed = format!("{new_raw_body}\n\n---\n# Team Instructions\n{instructions}");

    let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
    p.system_prompt = composed;

    let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", Some(instructions)).unwrap();
    assert!(
        result.ends_with("You are Paul, a wise orchestrator."),
        "new body not written: {result:?}"
    );
    assert!(
        !result.contains("# Team Instructions"),
        "Team Instructions must not appear in body: {result}"
    );
}

#[test]
fn test_prompt_edited_no_pack_instructions_body_rewritten_verbatim() {
    // No pack instructions: composed == raw. New body is system_prompt verbatim.
    let new_raw_body = "You are Paul, updated.";
    let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
    p.system_prompt = new_raw_body.to_string();

    let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", None).unwrap();
    assert!(
        result.ends_with("You are Paul, updated."),
        "body not updated: {result:?}"
    );
}

#[test]
fn test_prompt_unedited_body_preserved() {
    // system_prompt equals compose_prompt(current_raw_body, instructions) exactly.
    // The body section must not change even though frontmatter may be rewritten.
    let instructions = "Follow the rules.";
    let raw_body = "You are Paul.";
    let composed = format!("{raw_body}\n\n---\n# Team Instructions\n{instructions}");

    let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
    // Frontmatter also unchanged (matches PROMPT_MD).
    p.system_prompt = composed;

    let result = rewrite_persona_md(PROMPT_MD, &p, raw_body, Some(instructions)).unwrap();
    // Body must remain "You are Paul." (no prompt edit — body section preserved).
    assert!(
        result.ends_with("You are Paul.\n"),
        "body must be unchanged: {result:?}"
    );
    // Team Instructions must not leak into the body.
    assert!(
        !result.contains("# Team Instructions"),
        "Team Instructions must not appear: {result}"
    );
}

#[test]
fn test_prompt_safety_guard_missing_suffix_preserves_body() {
    // pack_instructions is non-empty but system_prompt does NOT end with the
    // expected suffix (user edited inside the Team Instructions block, or the
    // instructions drifted). The existing body must be preserved — no corruption.
    let instructions = "Follow the rules.";
    let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
    // system_prompt lacks the expected suffix entirely.
    p.system_prompt = "Some rogue prompt with # Team Instructions in the middle".to_string();

    let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", Some(instructions)).unwrap();
    // Body preserved from the file.
    assert!(
        result.ends_with("You are Paul.\n"),
        "body must be preserved by safety guard: {result:?}"
    );
}

#[test]
fn test_prompt_round_trip_no_double_append() {
    // After write-back, running compose_prompt on the written body + instructions
    // must reproduce the stored system_prompt exactly. This proves no double-append.
    let instructions = "Follow the rules.";
    let new_raw_body = "You are Paul, updated for the round-trip.";
    let composed = format!("{new_raw_body}\n\n---\n# Team Instructions\n{instructions}");

    let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
    p.system_prompt = composed.clone();

    let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", Some(instructions)).unwrap();

    // Extract the written body (everything after the last "---\n").
    let written_body = result.split("---\n").last().unwrap();
    // Re-compose: body + instructions must equal the original composed prompt.
    let recomposed = format!("{written_body}\n\n---\n# Team Instructions\n{instructions}");
    assert_eq!(
        recomposed, composed,
        "round-trip failed — double-append would occur: recomposed={recomposed:?}"
    );
}

#[test]
fn test_prompt_edited_with_trailing_newline_in_instructions() {
    // Regression: normal instructions.md files have a trailing newline.
    // compose_prompt includes it verbatim; system_prompt must NOT be trimmed
    // in update_persona or the suffix-strip fails and the safety guard fires.
    //
    // This test verifies that pack_instructions with a trailing "\n" still
    // decomposes correctly — i.e., the body is rewritten, not preserved.
    let instructions = "Follow the rules.\n"; // trailing newline from file read
    let new_raw_body = "You are Paul, rewritten.";
    let composed = format!("{new_raw_body}\n\n---\n# Team Instructions\n{instructions}");
    // system_prompt is NOT trimmed (update_persona must preserve it as-is).

    let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
    p.system_prompt = composed;

    let result = rewrite_persona_md(PROMPT_MD, &p, "You are Paul.", Some(instructions)).unwrap();
    assert!(
        result.contains("You are Paul, rewritten."),
        "body not rewritten with trailing-newline instructions: {result:?}"
    );
    assert!(
        !result.contains("# Team Instructions"),
        "Team Instructions must not appear in body: {result}"
    );
}

// ── write_back_persona_md path-resolution tests ───────────────────────────
//
// These tests verify that write_back_persona_md resolves the source file via
// the pack manifest rather than by convention. This is the class of bug that
// Thufir's IMPORTANT finding caught: a pack whose manifest points at
// `personas/foo.persona.md` (not `agents/<slug>.persona.md`) must still be
// rewritten correctly.

use tempfile::TempDir;

/// Build a minimal pack on disk with the given personas layout.
///
/// `persona_entries` is a list of (manifest_rel_path, file_content) pairs.
/// The manifest lists the relative paths; the files are written verbatim.
fn make_temp_pack(persona_entries: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    let root = dir.path();

    std::fs::create_dir_all(root.join(".plugin")).unwrap();
    let persona_paths: Vec<&str> = persona_entries.iter().map(|(p, _)| *p).collect();
    let manifest = serde_json::json!({
        "id": "test-team",
        "name": "Test Team",
        "version": "0.1.0",
        "personas": persona_paths,
    });
    std::fs::write(
        root.join(".plugin/plugin.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    for (rel_path, content) in persona_entries {
        let abs = root.join(rel_path);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, content).unwrap();
    }

    dir
}

#[test]
fn test_writeback_uses_manifest_path_not_convention() {
    // Pack uses `personas/paul.persona.md` layout (not `agents/`).
    // Convention-based path would derive `agents/paul.persona.md` (wrong).
    // Manifest-based resolution must write to the correct `personas/` path.
    let persona_md = "\
---
name: paul
display_name: \"Paul\"
description: \"Orchestrator\"
model: goose-claude-4-6-opus
---
You are Paul.
";
    let dir = make_temp_pack(&[("personas/paul.persona.md", persona_md)]);
    let source_dir = dir.path().to_path_buf();

    let pack = buzz_persona_pkg::pack::load_pack(&source_dir).unwrap();
    assert_eq!(pack.personas[0].name, "paul");
    let source_path = pack.personas[0].source_path.clone();
    // File lives under personas/, not agents/. Assert on path components so
    // the check is separator-agnostic (Windows uses `\`, not `/`).
    assert_eq!(
        source_path.file_name().and_then(|n| n.to_str()),
        Some("paul.persona.md"),
        "expected paul.persona.md: {}",
        source_path.display()
    );
    assert_eq!(
        source_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str()),
        Some("personas"),
        "expected personas/ layout: {}",
        source_path.display()
    );

    // Simulate what write_back_persona_md does after the fix:
    // use source_path from pack, not source_dir/agents/<slug>.persona.md
    let mut p = persona(
        "Paul Updated",
        Some("goose"),
        None,
        Some("databricks_v2"),
        Some("goose-claude-opus-4-8"),
    );
    p.source_team_persona_slug = Some("paul".to_string());

    let content = std::fs::read_to_string(&source_path).unwrap();
    let updated = rewrite_persona_md(&content, &p, "You are Paul.\n", None).unwrap();
    std::fs::write(&source_path, &updated).unwrap();

    let after = std::fs::read_to_string(&source_path).unwrap();
    assert!(
        after.contains("databricks_v2:goose-claude-opus-4-8"),
        "model not written: {after}"
    );
    assert!(
        after.ends_with("You are Paul.\n"),
        "body not preserved: {after:?}"
    );
}

#[test]
fn test_writeback_name_differs_from_filename() {
    // Pack file is `personas/orchestrator.persona.md` but `name: paul`.
    // Slug matches `name:`, not the filename.
    let persona_md = "\
---
name: paul
display_name: \"Paul\"
description: \"Orchestrator\"
model: old-model
---
You are Paul.
";
    let dir = make_temp_pack(&[("personas/orchestrator.persona.md", persona_md)]);
    let source_dir = dir.path().to_path_buf();

    let pack = buzz_persona_pkg::pack::load_pack(&source_dir).unwrap();
    let loaded = pack.personas.iter().find(|p| p.name == "paul").unwrap();
    let source_path = loaded.source_path.clone();
    // File basename is orchestrator, not paul
    assert!(
        source_path
            .to_string_lossy()
            .contains("orchestrator.persona.md"),
        "expected orchestrator.persona.md: {}",
        source_path.display()
    );

    let mut p = persona(
        "Paul",
        Some("goose"),
        None,
        Some("anthropic"),
        Some("claude-4"),
    );
    p.source_team_persona_slug = Some("paul".to_string());

    let content = std::fs::read_to_string(&source_path).unwrap();
    let updated = rewrite_persona_md(&content, &p, "You are Paul.\n", None).unwrap();
    std::fs::write(&source_path, &updated).unwrap();

    let after = std::fs::read_to_string(&source_path).unwrap();
    assert!(
        after.contains("anthropic:claude-4"),
        "model not written to orchestrator.persona.md: {after}"
    );
    // The wrong file (paul.persona.md in agents/) must NOT exist.
    assert!(
        !dir.path().join("agents/paul.persona.md").exists(),
        "convention-based path must not be created"
    );
}

// ── find_team_for_persona_source tests ────────────────────────────────────
//
// Verify that write_back_persona_md finds teams by team_persona_key, not
// team.id. Legacy/backfilled teams have a UUID `id` while AgentDefinition
// stores the manifest directory name in `source_team`; matching by `id`
// alone silently misses those teams.

fn make_team(id: &str, source_dir: Option<&str>) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        persona_ids: vec![],
        is_builtin: false,
        source_dir: source_dir.map(std::path::PathBuf::from),
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn make_team_with_path(id: &str, source_dir: &std::path::Path) -> TeamRecord {
    TeamRecord {
        id: id.to_string(),
        name: id.to_string(),
        description: None,
        persona_ids: vec![],
        is_builtin: false,
        source_dir: Some(source_dir.to_path_buf()),
        is_symlink: false,
        symlink_target: None,
        version: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn test_find_team_legacy_uuid_id_matched_by_source_dir_name() {
    // Legacy shape: team.id is a UUID; AgentDefinition.source_team is the
    // manifest directory name. The old `team.id == source_team` predicate
    // missed this — find_team_for_persona_source must match via source_dir.
    let teams = vec![make_team("some-uuid-123", Some("/teams/com.test.pack"))];
    let found = find_team_for_persona_source(&teams, "com.test.pack");
    assert!(
        found.is_some(),
        "legacy team must be found by manifest dir name, not UUID"
    );
    assert_eq!(found.unwrap().id, "some-uuid-123");
}

#[test]
fn test_find_team_modern_id_matched_directly() {
    // Modern shape: team.id equals the manifest directory name. Must match.
    let teams = vec![make_team("com.test.pack", Some("/teams/com.test.pack"))];
    let found = find_team_for_persona_source(&teams, "com.test.pack");
    assert!(found.is_some(), "modern team must be found");
}

#[test]
fn test_find_team_manifest_dir_name_matches_regardless_of_uuid_id() {
    // Regression: the old predicate `team.id == source_team` missed legacy
    // teams when source_team holds the manifest dir name, not the UUID.
    // This test confirms that searching by manifest dir name always works
    // even when team.id is a UUID.
    let teams = vec![make_team("some-uuid-123", Some("/teams/com.test.pack"))];
    // source_team holds the manifest dir name "com.test.pack", not the UUID.
    let by_dir = find_team_for_persona_source(&teams, "com.test.pack");
    assert!(
        by_dir.is_some(),
        "manifest dir name must find the legacy team"
    );
    assert_eq!(by_dir.unwrap().id, "some-uuid-123");
}

#[test]
fn test_find_team_no_source_dir_falls_back_to_id() {
    // JSON-only team: no source_dir, team_persona_key falls back to id.
    let teams = vec![make_team("builtin-team:fizz", None)];
    let found = find_team_for_persona_source(&teams, "builtin-team:fizz");
    assert!(
        found.is_some(),
        "no source_dir: must match via team.id fallback"
    );
}

#[test]
fn test_find_team_returns_none_when_no_match() {
    let teams = vec![make_team("some-uuid-123", Some("/teams/com.test.pack"))];
    let not_found = find_team_for_persona_source(&teams, "com.other.pack");
    assert!(not_found.is_none(), "unrelated source_team must not match");
}

// ── legacy symlink shape + missing-runtime insert ─────────────────────────
//
// Regression: write-back for a team whose `id` is a UUID (legacy backfill)
// and whose `source_dir` is a path whose FINAL component is the
// team_persona_key (e.g. `.../agents/teams/com.wpfleger.sietch-tabr`), with
// that path being a SYMLINK to the real pack directory. The persona's
// `source_team` stores the key, not the UUID, and the pack file has no
// `runtime:` frontmatter key (write-back must INSERT it).

/// Build a AgentDefinition matching the legacy incident shape.
fn legacy_persona(source_team: &str, runtime: Option<&str>) -> AgentDefinition {
    AgentDefinition {
        id: "ab5c038c-1b12-46e2-8283-d6f7c0606fce".to_string(),
        display_name: "Paul Updated".to_string(),
        avatar_url: None,
        system_prompt: "You are Paul.\n".to_string(),
        runtime: runtime.map(str::to_string),
        model: Some("claude-opus-4-8".to_string()),
        provider: Some("anthropic".to_string()),
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: Some(source_team.to_string()),
        source_team_persona_slug: Some("paul".to_string()),
        env_vars: BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        mcp_toolsets: None,
        parallelism: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

#[test]
fn test_writeback_legacy_symlink_uuid_team_inserts_missing_runtime() {
    // Real-world shape: pack lives in a real directory (pack_dir), and the
    // team's source_dir is a SYMLINK at `<link_parent>/com.wpfleger.sietch-tabr`
    // pointing to pack_dir. The team record has a UUID id; persona.source_team
    // is the link basename (the team_persona_key), not the UUID.
    let pack_dir = TempDir::new().expect("pack_dir tempdir");
    let link_parent = TempDir::new().expect("link_parent tempdir");

    // paul.persona.md has NO runtime: key (write-back must INSERT it).
    let persona_md = "\
---
name: paul
display_name: \"Paul\"
description: \"Orchestrator\"
---
You are Paul.
";
    // Build a minimal pack in pack_dir.
    std::fs::create_dir_all(pack_dir.path().join(".plugin")).unwrap();
    let manifest = serde_json::json!({
        "id": "com.wpfleger.sietch-tabr",
        "name": "Sietch Tabr",
        "version": "0.1.0",
        "personas": ["paul.persona.md"],
    });
    std::fs::write(
        pack_dir.path().join(".plugin/plugin.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(pack_dir.path().join("paul.persona.md"), persona_md).unwrap();

    // Create the symlink: `<link_parent>/com.wpfleger.sietch-tabr` → pack_dir
    let symlink_path = link_parent.path().join("com.wpfleger.sietch-tabr");
    #[cfg(unix)]
    std::os::unix::fs::symlink(pack_dir.path(), &symlink_path).expect("symlink");
    #[cfg(not(unix))]
    {
        // On non-Unix, skip symlink traversal test (macOS-only scenario).
        return;
    }

    // Team: UUID id, source_dir = symlink path (basename = team_persona_key).
    let team = TeamRecord {
        id: "ab5c038c-1b12-46e2-8283-d6f7c0606fce".to_string(),
        name: "Sietch Tabr".to_string(),
        description: None,
        persona_ids: vec![],
        is_builtin: false,
        source_dir: Some(symlink_path.clone()),
        is_symlink: true,
        symlink_target: Some(pack_dir.path().to_string_lossy().to_string()),
        version: None,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_at: "2026-01-01T00:00:00Z".to_string(),
    };
    let teams = vec![team];

    // persona.source_team = the basename = team_persona_key
    let persona = legacy_persona("com.wpfleger.sietch-tabr", Some("goose"));

    let result = try_write_back_persona_md(&teams, &persona);
    assert!(
        result.is_ok(),
        "write-back through symlink must succeed: {result:?}"
    );

    // Verify runtime was inserted into the pack file.
    let written = std::fs::read_to_string(pack_dir.path().join("paul.persona.md")).unwrap();
    assert!(
        written.contains("runtime: goose"),
        "runtime must be inserted into the file: {written}"
    );
    assert!(
        written.contains("display_name:"),
        "display_name must be present: {written}"
    );
}

#[cfg(unix)]
#[test]
fn test_writeback_readonly_pack_file_returns_warning() {
    use std::os::unix::fs::PermissionsExt;

    // Pack with a persona that has a different display_name than what we'll
    // request — this forces a write (not a no-op short-circuit).
    let persona_md = "\
---
name: paul
display_name: \"Paul Old\"
description: \"Orchestrator\"
runtime: goose
---
You are Paul.
";
    let dir = make_temp_pack(&[("paul.persona.md", persona_md)]);
    let source_dir = dir.path().to_path_buf();

    // Make the file read-only.
    let paul_path = dir.path().join("paul.persona.md");
    let mut perms = std::fs::metadata(&paul_path).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&paul_path, perms).unwrap();

    let team = make_team_with_path("test-team", &source_dir);
    let teams = vec![team];

    // Persona with updated display_name — write is needed but will be denied.
    let mut persona = persona("Paul New", Some("goose"), None, None, Some("some-model"));
    persona.source_team = Some("test-team".to_string());
    persona.source_team_persona_slug = Some("paul".to_string());

    let result = try_write_back_persona_md(&teams, &persona);
    assert!(
        result.is_err(),
        "read-only pack file must return an error: {result:?}"
    );
    let warning = result.unwrap_err();
    assert!(
        warning.contains("write"),
        "error must mention the write failure: {warning}"
    );
}

#[test]
fn test_multi_save_round_trip_short_circuit_holds_both_times() {
    // Regression: the `==` short-circuit must hold on the second save too.
    //
    // Without the frontend fix (removing systemPrompt.trim()), the first
    // save stores the trimmed composed prompt; on re-open the form emits the
    // trimmed value, which != compose_prompt(body, instructions\n) — the `==`
    // check fails, suffix-strip fails on the trimmed string, and the safety
    // guard fires on every subsequent save. This test verifies the path stays
    // clean across two consecutive no-edit saves.
    let instructions = "Follow the rules.\n"; // realistic: trailing newline from file
                                              // `split_frontmatter` strips the "\n" after the closing "---" delimiter
                                              // but preserves the rest of the body verbatim. PROMPT_MD's body line ends
                                              // with "\n", so loaded.prompt at runtime = "You are Paul.\n".
    let (_, raw_body) = buzz_persona_pkg::persona::split_frontmatter(PROMPT_MD).unwrap();
    // compose_prompt equivalent (must match the real impl exactly)
    let composed = format!("{raw_body}\n\n---\n# Team Instructions\n{instructions}");

    // ── Save 1: user opens dialog, saves without editing ─────────────────
    let mut p = persona("Paul", None, None, None, Some("goose-claude-4-6-opus"));
    p.system_prompt = composed.clone(); // no frontend trim — system_prompt is the full composed value

    let written_1 = rewrite_persona_md(PROMPT_MD, &p, raw_body, Some(instructions)).unwrap();
    // Short-circuit must have fired: body is preserved (still "You are Paul.\n").
    assert!(
        written_1.ends_with("You are Paul.\n"),
        "save 1: body must be preserved by == short-circuit: {written_1:?}"
    );
    assert!(
        !written_1.contains("# Team Instructions"),
        "save 1: Team Instructions must not appear in file body: {written_1}"
    );

    // ── Save 2: user re-opens (reads written_1), saves again without editing
    // `loaded.prompt` comes from split_frontmatter on the written file — same
    // raw_body as before (the write-back preserved it byte-for-byte).
    let (_, body_from_file) = buzz_persona_pkg::persona::split_frontmatter(&written_1).unwrap();
    assert_eq!(
        body_from_file, raw_body,
        "split_frontmatter after save 1 must return the original raw_body"
    );

    // Compose again from the read-back body — must equal the stored system_prompt.
    let recomposed = format!("{body_from_file}\n\n---\n# Team Instructions\n{instructions}");
    assert_eq!(
        recomposed, composed,
        "save 2 precondition: recomposed must equal stored system_prompt so == holds"
    );

    // Now simulate save 2: system_prompt is the recomposed value (no trim).
    p.system_prompt = recomposed;
    let written_2 = rewrite_persona_md(&written_1, &p, body_from_file, Some(instructions)).unwrap();
    // Short-circuit must fire again: body still preserved, no corruption.
    assert!(
        written_2.ends_with("You are Paul.\n"),
        "save 2: body must still be preserved by == short-circuit: {written_2:?}"
    );
    assert!(
        !written_2.contains("# Team Instructions"),
        "save 2: Team Instructions must not appear after second save: {written_2}"
    );
    // Both writes are byte-identical — no spurious mutations.
    assert_eq!(
        written_1, written_2,
        "save 2: file content must be identical to save 1"
    );
}
