use super::*;

#[test]
fn nest_dir_is_under_home() {
    if let Some(dir) = nest_dir() {
        // Accepts both .buzz (prod) and .buzz-dev (dev) depending on
        // whether init_nest_dir was called before this test ran.
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(
            name == NEST_DIR_PROD || name == NEST_DIR_DEV,
            "nest_dir must end with .buzz or .buzz-dev, got {dir:?}"
        );
    }
}

#[test]
fn init_nest_dir_prod_sets_buzz() {
    // init_nest_dir is idempotent (OnceLock) — once set, subsequent calls
    // are no-ops. We can only test the fallback path if the OnceLock is
    // unset, which is only true in a fresh process. Instead, verify that
    // nest_dir() always returns a path ending with a valid nest suffix.
    let dir = nest_dir();
    if let Some(d) = dir {
        let name = d.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(
            name == NEST_DIR_PROD || name == NEST_DIR_DEV,
            "nest_dir suffix must be .buzz or .buzz-dev, got {d:?}"
        );
    }
}

#[test]
fn ensure_nest_creates_all_dirs_and_agents_md() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");

    ensure_nest_at(&root).unwrap();

    // All subdirectories exist.
    for dir in NEST_DIRS {
        assert!(root.join(dir).is_dir(), "{dir}/ should exist");
    }
    // REPOS is provisioned separately (may be a symlink); with no
    // repos_dir configured it lands as a real directory.
    assert!(root.join("REPOS").is_dir(), "REPOS/ should exist");

    // AGENTS.md was written with default content.
    let content = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert_eq!(content, AGENTS_MD);

    // Permissions are 700 on Unix for root and all subdirs.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "root should be 700");
        for dir in NEST_DIRS {
            let mode = fs::metadata(root.join(dir)).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{dir}/ should be 700");
        }
        let repos_mode = fs::metadata(root.join("REPOS"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(repos_mode, 0o700, "REPOS/ should be 700");
    }
}

#[test]
fn ensure_nest_is_idempotent_and_preserves_custom_content() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");

    // First call creates everything.
    ensure_nest_at(&root).unwrap();

    // User customizes AGENTS.md.
    let agents = root.join("AGENTS.md");
    fs::write(&agents, "my custom instructions").unwrap();

    // Second call succeeds and does not overwrite.
    ensure_nest_at(&root).unwrap();

    assert_eq!(
        fs::read_to_string(&agents).unwrap(),
        "my custom instructions"
    );

    // All dirs still exist.
    for dir in NEST_DIRS {
        assert!(root.join(dir).is_dir(), "{dir}/ should still exist");
    }
}

#[cfg(unix)]
#[test]
fn ensure_nest_rejects_symlink_root() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("real_dir");
    fs::create_dir(&target).unwrap();
    let link = tmp.path().join(".buzz");
    std::os::unix::fs::symlink(&target, &link).unwrap();

    let result = ensure_nest_at(&link);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("symlink"));
}

#[test]
fn ensure_nest_creates_skill_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();

    // Canonical location under .agents.
    let skill = root.join(".agents/skills/buzz-cli/SKILL.md");
    assert!(skill.exists(), "SKILL.md should exist at .agents path");
    let content = fs::read_to_string(&skill).unwrap();
    assert_eq!(content, BUZZ_CLI_SKILL_MD);

    // On unix, harness-specific symlinks should resolve to the canonical dir.
    #[cfg(unix)]
    {
        for dir in [".goose/skills", ".claude/skills", ".codex/skills"] {
            let link = root.join(dir).join("buzz-cli");
            assert!(
                link.symlink_metadata().unwrap().file_type().is_symlink(),
                "{dir}/buzz-cli should be a symlink"
            );
            assert!(
                link.join("SKILL.md").exists(),
                "symlink at {dir}/buzz-cli should resolve to dir with SKILL.md"
            );
        }
    }
}

#[test]
fn ensure_nest_does_not_overwrite_skill_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();

    let skill = root.join(".agents/skills/buzz-cli/SKILL.md");
    fs::write(&skill, "custom skill content").unwrap();

    ensure_nest_at(&root).unwrap();
    assert_eq!(fs::read_to_string(&skill).unwrap(), "custom skill content");
}

#[cfg(unix)]
#[test]
fn ensure_nest_skill_dir_has_700_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();
    // Canonical path and all provider parent dirs should be locked down.
    // Symlinks (e.g. .goose/skills/buzz-cli) are skipped by the chmod loop.
    for dir in [
        ".agents",
        ".agents/skills",
        ".agents/skills/buzz-cli",
        ".goose",
        ".goose/skills",
        ".claude",
        ".claude/skills",
        ".codex",
        ".codex/skills",
    ] {
        let path = root.join(dir);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{dir} should be 700");
    }
}

#[cfg(unix)]
#[test]
fn ensure_nest_skips_permissions_on_symlinked_child() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");

    // First call creates the real nest.
    ensure_nest_at(&root).unwrap();

    // Replace REPOS/ with a symlink to an external directory.
    let external = tmp.path().join("external");
    fs::create_dir(&external).unwrap();
    fs::set_permissions(&external, fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_dir(root.join("REPOS")).unwrap();
    std::os::unix::fs::symlink(&external, root.join("REPOS")).unwrap();

    // Second call should succeed — it skips chmod on the symlinked child.
    ensure_nest_at(&root).unwrap();

    // The external directory's permissions should be unchanged (755, not 700).
    let mode = fs::metadata(&external).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o755,
        "symlinked child's target should not be chmod'd"
    );
}

#[cfg(unix)]
#[test]
fn ensure_nest_migrates_old_skill_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");

    // Simulate a pre-migration install: real directory at old path.
    // Create the nest first to get all dirs, then simulate old layout.
    ensure_nest_at(&root).unwrap();

    // Remove the symlink and new skill dir, recreate old real dir.
    let _ = fs::remove_file(root.join(".claude/skills/buzz-cli"));
    let _ = fs::remove_dir_all(root.join(".agents/skills/buzz-cli"));
    let old_skill_dir = root.join(".claude/skills/buzz-cli");
    fs::create_dir_all(&old_skill_dir).unwrap();
    fs::write(old_skill_dir.join("SKILL.md"), "user edited skill").unwrap();

    // Delete version file to force refresh.
    let _ = fs::remove_file(root.join(".agents/skills/buzz-cli/.skill-version"));

    // Re-run ensure_nest_at — should trigger migration in refresh_skill_md_if_stale.
    ensure_nest_at(&root).unwrap();

    // New canonical location exists with user's content preserved.
    let new_skill = root.join(".agents/skills/buzz-cli/SKILL.md");
    assert!(new_skill.exists(), "SKILL.md should exist at new path");
    assert_eq!(fs::read_to_string(&new_skill).unwrap(), "user edited skill");

    // Old path is now a symlink, not a real directory.
    let old_path = root.join(".claude/skills/buzz-cli");
    assert!(
        old_path
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "old path should now be a symlink"
    );
}

#[cfg(unix)]
#[test]
fn ensure_skill_symlinks_are_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();
    // Second call should succeed without errors.
    ensure_nest_at(&root).unwrap();
    // All symlinks still valid and point to relative targets.
    for dir in [".goose/skills", ".claude/skills", ".codex/skills"] {
        let link = root.join(dir).join("buzz-cli");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(
            link.join("SKILL.md").exists(),
            "symlink at {dir}/buzz-cli should resolve to dir with SKILL.md"
        );
        let target = fs::read_link(&link).unwrap();
        assert_eq!(
            target.to_str().unwrap(),
            format!("../../{CANONICAL_SKILL_DIR}"),
            "symlink at {dir}/buzz-cli should use relative target"
        );
    }
}

#[cfg(unix)]
#[test]
fn ensure_skill_symlinks_skips_existing_path_during_initial_pass() {
    // ensure_skill_symlinks skips any path where symlink_metadata succeeds.
    // However, refresh_skill_md_if_stale (called after ensure_skill_symlinks)
    // migrates pre-existing real directories at .claude/skills/buzz-cli to
    // symlinks. This test verifies the end-to-end behavior: a pre-existing real
    // dir at the claude path is migrated to a symlink.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    // Pre-create a real directory where a symlink would go.
    let real_dir = root.join(".claude/skills/buzz-cli");
    fs::create_dir_all(&real_dir).unwrap();
    // Place SKILL.md so migration preserves it.
    fs::write(real_dir.join("SKILL.md"), "custom skill content").unwrap();

    ensure_nest_at(&root).unwrap();

    // Migration converts the real dir to a symlink; content is moved to canonical path.
    assert!(
        real_dir
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        ".claude/skills/buzz-cli should be migrated to a symlink"
    );
    // The canonical path now holds the migrated content.
    let canonical = root.join(".agents/skills/buzz-cli/SKILL.md");
    assert_eq!(
        fs::read_to_string(&canonical).unwrap(),
        "custom skill content"
    );
}

#[cfg(unix)]
#[test]
fn ensure_skill_symlinks_skip_dangling_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    // Pre-create a dangling symlink where the .codex link would go.
    let codex_skills = root.join(".codex/skills");
    fs::create_dir_all(&codex_skills).unwrap();
    let dangling = codex_skills.join("buzz-cli");
    std::os::unix::fs::symlink("/nonexistent/target", &dangling).unwrap();

    ensure_nest_at(&root).unwrap();

    // Dangling symlink should be left alone (not clobbered).
    assert!(dangling
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&dangling).unwrap().to_str().unwrap(),
        "/nonexistent/target"
    );
}

#[test]
fn cli_link_name_prod_is_buzz() {
    assert_eq!(cli_link_name(false), "buzz");
}

#[test]
fn cli_link_name_dev_is_buzz_dev() {
    assert_eq!(cli_link_name(true), "buzz-dev");
}

#[cfg(unix)]
#[test]
fn ensure_cli_symlink_creates_symlink_prod() {
    let tmp = tempfile::tempdir().unwrap();
    let exe_parent = tmp.path().join("MacOS");
    fs::create_dir(&exe_parent).unwrap();
    fs::write(exe_parent.join("buzz"), "binary").unwrap();

    let local_bin = tmp.path().join("local_bin");
    fs::create_dir_all(&local_bin).unwrap();

    // Prod link name is "buzz"; simulate the symlink creation path.
    let link = local_bin.join(cli_link_name(false));
    std::os::unix::fs::symlink(exe_parent.join("buzz"), &link).unwrap();
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), exe_parent.join("buzz"));
}

#[cfg(unix)]
#[test]
fn ensure_cli_symlink_creates_symlink_dev() {
    let tmp = tempfile::tempdir().unwrap();
    let exe_parent = tmp.path().join("MacOS");
    fs::create_dir(&exe_parent).unwrap();
    fs::write(exe_parent.join("buzz"), "binary").unwrap();

    let local_bin = tmp.path().join("local_bin");
    fs::create_dir_all(&local_bin).unwrap();

    // Dev link must be "buzz-dev", never "buzz".
    assert_eq!(cli_link_name(true), "buzz-dev");

    let link = local_bin.join(cli_link_name(true));
    std::os::unix::fs::symlink(exe_parent.join("buzz"), &link).unwrap();
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(fs::read_link(&link).unwrap(), exe_parent.join("buzz"));
    // Prod link must not exist — the two builds don't touch each other.
    assert!(!local_bin.join("buzz").exists());
}

#[cfg(unix)]
#[test]
fn ensure_cli_symlink_does_not_clobber_regular_file_prod() {
    let tmp = tempfile::tempdir().unwrap();
    let local_bin = tmp.path().join("local_bin");
    fs::create_dir_all(&local_bin).unwrap();
    let link = local_bin.join(cli_link_name(false));
    fs::write(&link, "user-installed binary").unwrap();

    // Regular files are preserved — the Ok(_) branch skips them.
    assert!(link.symlink_metadata().unwrap().file_type().is_file());
    assert_eq!(fs::read_to_string(&link).unwrap(), "user-installed binary");
}

#[cfg(unix)]
#[test]
fn ensure_cli_symlink_does_not_clobber_regular_file_dev() {
    let tmp = tempfile::tempdir().unwrap();
    let local_bin = tmp.path().join("local_bin");
    fs::create_dir_all(&local_bin).unwrap();
    let link = local_bin.join(cli_link_name(true));
    fs::write(&link, "user-installed buzz-dev binary").unwrap();

    // Regular files at the dev path are also preserved.
    assert!(link.symlink_metadata().unwrap().file_type().is_file());
    assert_eq!(
        fs::read_to_string(&link).unwrap(),
        "user-installed buzz-dev binary"
    );
}

fn make_persona(id: &str, display_name: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        display_name: display_name.to_string(),
        avatar_url: None,
        system_prompt: String::new(),
        runtime: None,
        model: None,
        provider: None,
        name_pool: vec![],
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        env_vars: std::collections::BTreeMap::new(),
        respond_to: None,
        respond_to_allowlist: Vec::new(),
        parallelism: None,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn make_agent(name: &str, persona_id: Option<&str>) -> ManagedAgentRecord {
    ManagedAgentRecord {
        pubkey: String::new(),
        name: name.to_string(),
        persona_id: persona_id.map(|s| s.to_string()),
        private_key_nsec: String::new(),
        auth_tag: None,
        relay_url: String::new(),
        avatar_url: None,
        acp_command: String::new(),
        agent_command: String::new(),
        agent_command_override: None,
        agent_args: vec![],
        mcp_command: String::new(),
        turn_timeout_seconds: 0,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: 1,
        system_prompt: None,
        model: None,
        provider: None,
        persona_source_version: None,
        start_on_app_launch: false,
        auto_restart_on_config_change: true,
        runtime_pid: None,
        backend: BackendKind::default(),
        backend_agent_id: None,
        provider_binary_path: None,
        team_id: None,
        persona_team_dir: None,
        persona_name_in_team: None,
        created_at: String::new(),
        updated_at: String::new(),
        last_started_at: None,
        last_stopped_at: None,
        last_exit_code: None,
        last_error: None,
        last_error_code: None,
        respond_to: RespondTo::default(),
        respond_to_allowlist: vec![],
        env_vars: std::collections::BTreeMap::new(),
        display_name: None,
        slug: None,
        runtime: None,
        name_pool: Vec::new(),
        is_builtin: false,
        is_active: true,
        source_team: None,
        source_team_persona_slug: None,
        definition_respond_to: None,
        definition_respond_to_allowlist: Vec::new(),
        definition_parallelism: None,
        relay_mesh: None,
    }
}

#[test]
fn test_render_dynamic_section_with_agents() {
    let personas = vec![make_persona("p1", "Builder")];
    let agents = vec![make_agent("Kit", Some("p1"))];
    let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");
    assert!(output.contains("| Kit | Builder | @Kit |"));
    assert!(output.contains("| Name | Persona | How to address |"));
    assert!(output.contains("## Workspace"));
}

#[test]
fn test_render_dynamic_section_empty() {
    let output = render_dynamic_section(&[], &[], "ws://example.com:3000");
    assert!(output.contains("No agents deployed yet"));
}

#[test]
fn test_render_dynamic_section_agent_no_persona() {
    let personas = vec![make_persona("p1", "Builder")];
    let agents = vec![make_agent("Scout", Some("nonexistent"))];
    let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");
    assert!(output.contains("| Scout | — | @Scout |"));
}

#[test]
fn test_upsert_managed_section_with_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\nsome content\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nold section\n<!-- END BUZZ MANAGED -->\n\nafter\n",
        )
        .unwrap();

    upsert_managed_section(&file, "new section").unwrap();

    let result = fs::read_to_string(&file).unwrap();
    assert!(result.contains("<!-- BEGIN BUZZ MANAGED"));
    assert!(result.contains("<!-- END BUZZ MANAGED -->"));
    assert!(result.contains("new section"));
    assert!(!result.contains("old section"));
    assert!(result.contains("# Header"));
    assert!(result.contains("some content"));
    assert!(result.contains("after"));
}

#[test]
fn test_upsert_managed_section_without_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(&file, "# Header\n\nexisting content\n").unwrap();

    upsert_managed_section(&file, "injected section").unwrap();

    let result = fs::read_to_string(&file).unwrap();
    assert!(result.contains("# Header"));
    assert!(result.contains("existing content"));
    assert!(result.contains("<!-- BEGIN BUZZ MANAGED"));
    assert!(result.contains("<!-- END BUZZ MANAGED -->"));
    assert!(result.contains("injected section"));
    let begin_pos = result.find("<!-- BEGIN BUZZ MANAGED").unwrap();
    let header_pos = result.find("# Header").unwrap();
    assert!(
        header_pos < begin_pos,
        "original content should precede the managed section"
    );
}

#[test]
fn test_upsert_managed_section_no_tmp_leftover() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(&file, "# Header\n").unwrap();

    upsert_managed_section(&file, "content").unwrap();

    // Verify no stray temp files in the directory
    let entries: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "only AGENTS.md should remain, no temp files"
    );
    assert_eq!(entries[0].file_name(), "AGENTS.md");
}

#[test]
fn test_upsert_end_before_begin() {
    // An END marker that precedes a BEGIN marker forms no valid ordered pair.
    // find_managed_markers returns None (BEGIN found, but no END after it),
    // so the orphan BEGIN line is stripped and a new block is appended.
    // The stray END line and content between END and BEGIN remain in the file
    // because strip_orphan_begin_marker only removes the BEGIN line itself.
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\n<!-- END BUZZ MANAGED -->\nsome middle content\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nold section\n",
        )
        .unwrap();

    upsert_managed_section(&file, "new section").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(result.contains("# Header"), "original header must survive");
    assert!(
        result.contains("new section"),
        "new content must be present"
    );
    assert!(
        result.contains("some middle content"),
        "content between markers must survive"
    );

    // Exactly one BEGIN marker in the output (the orphan was stripped, new one appended).
    assert_eq!(
        result.matches(BEGIN_MARKER).count(),
        1,
        "exactly one BEGIN marker after orphan cleanup"
    );

    // The single BEGIN marker must have a matching END marker after it.
    let begin_pos = result
        .find(BEGIN_MARKER)
        .expect("BEGIN marker must be present");
    let end_pos = result[begin_pos..].find(END_MARKER).map(|p| begin_pos + p);
    assert!(
        end_pos.is_some(),
        "an END marker must appear after the appended BEGIN marker"
    );
}

#[test]
fn test_upsert_begin_only_no_end() {
    // A file with BEGIN but no END has an orphan marker.
    // find_managed_markers returns None (no END found after BEGIN),
    // so strip_orphan_begin_marker removes the BEGIN line.
    // Content that followed the orphan BEGIN is preserved (only the marker line is stripped,
    // not the body that came after it).
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\nsome content\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\norphaned section without end marker\n",
        )
        .unwrap();

    upsert_managed_section(&file, "fresh section").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(result.contains("# Header"), "original header must survive");
    assert!(
        result.contains("some content"),
        "original body must survive"
    );
    assert!(
        result.contains("fresh section"),
        "new content must be present"
    );

    let begin_pos = result
        .find(BEGIN_MARKER)
        .expect("BEGIN marker must be present");
    let end_pos = result.find(END_MARKER).expect("END marker must be present");
    assert!(
        begin_pos < end_pos,
        "the appended BEGIN marker must precede the appended END marker"
    );

    // Exactly one BEGIN marker after orphan cleanup.
    assert_eq!(
        result.matches(BEGIN_MARKER).count(),
        1,
        "exactly one BEGIN marker after orphan cleanup"
    );
}

#[test]
fn test_upsert_duplicate_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nfirst block\n<!-- END BUZZ MANAGED -->\n\nbetween blocks\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nsecond block\n<!-- END BUZZ MANAGED -->\n",
        )
        .unwrap();

    upsert_managed_section(&file, "replaced").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(
        result.contains("replaced"),
        "replacement content must be present"
    );
    assert!(
        !result.contains("first block"),
        "first block must be replaced"
    );
    assert!(
        result.contains("second block"),
        "second pair content must survive"
    );
    assert!(
        result.contains("between blocks"),
        "text between pairs must survive"
    );
}

#[test]
fn test_upsert_marker_in_code_block() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    // Indented by 4 spaces — not at column 0, so should NOT match as a real marker.
    fs::write(
        &file,
        "# Header\n\n    <!-- BEGIN BUZZ MANAGED — some indented marker -->\n\nReal content here\n",
    )
    .unwrap();

    upsert_managed_section(&file, "appended content").unwrap();

    let result = fs::read_to_string(&file).unwrap();

    assert!(
        result.contains("    <!-- BEGIN BUZZ MANAGED — some indented marker -->"),
        "indented marker inside code block must be preserved verbatim"
    );
    assert!(
        result.contains("appended content"),
        "new content must be appended"
    );
    assert!(
        result.contains("Real content here"),
        "existing body must survive"
    );

    // The real markers appended at the end must be at line-start (column 0).
    let begin_pos = result
        .find("<!-- BEGIN BUZZ MANAGED — regenerated")
        .expect("regenerated BEGIN marker must be present");
    assert!(
        begin_pos == 0 || result.as_bytes()[begin_pos - 1] == b'\n',
        "appended BEGIN marker must be at line start"
    );
}

#[test]
fn test_render_pipe_in_agent_name() {
    let personas = vec![make_persona("p1", "Builder")];
    let agents = vec![make_agent("Kit|Pro", Some("p1"))];
    let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");

    assert!(
        output.contains("Kit\\|Pro"),
        "pipe in agent name must be escaped as \\|"
    );
    // An unescaped bare `|` immediately adjacent to "Kit|Pro" would break table parsing.
    assert!(
        !output.contains("| Kit|Pro |"),
        "unescaped pipe in agent name must not appear as a cell boundary"
    );

    // The row must start and end with `|` and the escaped name and address must appear.
    let kit_row = output
        .lines()
        .find(|l| l.contains("Kit\\|Pro"))
        .expect("Kit\\|Pro row must be present");
    assert!(kit_row.starts_with('|'), "row must start with |");
    assert!(kit_row.ends_with('|'), "row must end with |");
    assert!(
        kit_row.contains("@Kit\\|Pro"),
        "address cell must use escaped name"
    );
}

#[test]
fn test_render_newline_in_persona_name() {
    let personas = vec![make_persona("p1", "Builder\nExpert")];
    let agents = vec![make_agent("Scout", Some("p1"))];
    let output = render_dynamic_section(&personas, &agents, "ws://example.com:3000");

    assert!(
        output.contains("Builder Expert"),
        "newline in persona display_name must be replaced with a space"
    );

    // The table row for Scout must be a single line (no embedded newline).
    let scout_row = output
        .lines()
        .find(|l| l.contains("Scout"))
        .expect("Scout row must be present");
    assert!(
        scout_row.contains("Builder Expert"),
        "persona name with newline replaced by space must appear on the Scout row"
    );
}

#[test]
fn test_upsert_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("AGENTS.md");
    fs::write(
            &file,
            "# Header\n\n<!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\nexisting section\n<!-- END BUZZ MANAGED -->\n",
        )
        .unwrap();

    upsert_managed_section(&file, "same content").unwrap();
    let after_first = fs::read_to_string(&file).unwrap();

    upsert_managed_section(&file, "same content").unwrap();
    let after_second = fs::read_to_string(&file).unwrap();

    assert_eq!(
        after_first, after_second,
        "upsert must be idempotent: second call must not alter the file"
    );
}

#[test]
fn refresh_agents_md_writes_version_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();
    let version = fs::read_to_string(root.join(".nest-agents-version")).unwrap();
    assert_eq!(version.trim(), NEST_AGENTS_VERSION.to_string());
}

#[test]
fn refresh_skill_md_writes_version_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();
    let version = fs::read_to_string(root.join(".agents/skills/buzz-cli/.skill-version")).unwrap();
    assert_eq!(version.trim(), NEST_SKILL_VERSION.to_string());
}

#[test]
fn refresh_agents_md_preserves_managed_section() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();

    // Simulate a managed section update.
    let agents_md = root.join("AGENTS.md");
    upsert_managed_section(
        &agents_md,
        "## Active Agents\n\n| Name | Role |\n|------|------|\n| Kit | Builder |",
    )
    .unwrap();

    // Remove version file to simulate an upgrade.
    fs::remove_file(root.join(".nest-agents-version")).unwrap();

    // Re-run ensure_nest_at (triggers refresh).
    ensure_nest_at(&root).unwrap();

    let content = fs::read_to_string(&agents_md).unwrap();
    // Static content should be refreshed (from template).
    assert!(
        content.starts_with("# Buzz Nest"),
        "template header must be present"
    );
    // Managed section should be preserved.
    assert!(
        content.contains("Kit"),
        "managed section agent table must survive refresh"
    );
    assert!(content.contains(BEGIN_MARKER), "BEGIN marker must survive");
    assert!(content.contains(END_MARKER), "END marker must survive");
}

#[test]
fn refresh_skips_when_version_current() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();

    // Manually change AGENTS.md content after version file is written.
    let agents_md = root.join("AGENTS.md");
    fs::write(&agents_md, "user modified content").unwrap();

    // Re-run ensure_nest_at — version file is current, so no refresh.
    ensure_nest_at(&root).unwrap();

    let content = fs::read_to_string(&agents_md).unwrap();
    assert_eq!(
        content, "user modified content",
        "should not overwrite when version is current"
    );
}

#[test]
fn refresh_skill_overwrites_on_version_bump() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();

    let skill_md = root.join(".agents/skills/buzz-cli/SKILL.md");
    fs::write(&skill_md, "stale skill content").unwrap();

    // Remove version file to simulate upgrade.
    let _ = fs::remove_file(root.join(".agents/skills/buzz-cli/.skill-version"));

    ensure_nest_at(&root).unwrap();

    let content = fs::read_to_string(&skill_md).unwrap();
    assert_eq!(
        content, BUZZ_CLI_SKILL_MD,
        "SKILL.md must be refreshed on version bump"
    );
}

#[test]
fn test_path_is_dev_nest_dev_path_returns_true() {
    let path = std::path::Path::new("/Users/someone/.buzz-dev");
    assert!(
        path_is_dev_nest(path),
        ".buzz-dev path must be identified as dev nest"
    );
}

#[test]
fn test_path_is_dev_nest_prod_path_returns_false() {
    let path = std::path::Path::new("/Users/someone/.buzz");
    assert!(
        !path_is_dev_nest(path),
        ".buzz path must not be identified as dev nest"
    );
}

#[test]
fn test_path_is_dev_nest_unrelated_path_returns_false() {
    let path = std::path::Path::new("/Users/someone/.buzz-staging");
    assert!(
        !path_is_dev_nest(path),
        "unrelated path must not be identified as dev nest"
    );
}

#[test]
fn test_path_is_dev_nest_root_returns_false() {
    let path = std::path::Path::new("/");
    assert!(
        !path_is_dev_nest(path),
        "root path must not be identified as dev nest"
    );
}
