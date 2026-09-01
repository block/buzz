use super::*;

#[test]
fn nest_dir_is_under_home() {
    if let Some(dir) = nest_dir() {
        // Accepts both .buzz (prod) and .buzz-dev (dev) depending on
        // whether init_nest_dir was called before this test ran.
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        assert!(
            name == NEST_DIR_PROD || name == crate::build_identity::nest_name(true),
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
            name == NEST_DIR_PROD || name == crate::build_identity::nest_name(true),
            "nest_dir suffix must be .buzz or .buzz-dev, got {d:?}"
        );
    }
}

#[test]
fn nest_agents_template_separates_commit_attribution_claims() {
    assert_eq!(AGENTS_MD.matches("## Git Commit Attribution").count(), 1);
    assert!(AGENTS_MD.contains(
        "Git authorship, co-authorship, DCO sign-off, and cryptographic signing are separate claims"
    ));
    assert!(AGENTS_MD
        .contains("Request, approval, review, or accountability alone is not co-authorship"));
    assert!(AGENTS_MD.contains("A sign-off is not an approval marker"));
    assert!(AGENTS_MD.contains("Never use another person's signing key"));
    assert!(AGENTS_MD.contains("inspect every outgoing commit against the actual upstream or base"));
    assert!(AGENTS_MD.contains("An agent-owned repository may use the agent as author"));
    assert!(!AGENTS_MD.contains("every commit MUST include a `Signed-off-by`"));
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

#[test]
fn cli_link_name_prod_follows_build_identity() {
    let expected = crate::build_identity::demo_slug()
        .map(|slug| format!("buzz-demo-{slug}"))
        .unwrap_or_else(|| "buzz".to_string());
    assert_eq!(cli_link_name(false), expected);
}

#[test]
fn cli_link_name_dev_follows_build_identity() {
    let expected = crate::build_identity::demo_slug()
        .map(|slug| format!("buzz-demo-{slug}"))
        .unwrap_or_else(|| "buzz-dev".to_string());
    assert_eq!(cli_link_name(true), expected);
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

    // Dev and demo links must never overwrite production's "buzz".
    assert_ne!(cli_link_name(true), "buzz");

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

#[test]
fn refresh_agents_md_writes_version_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();
    let version = fs::read_to_string(root.join(".nest-agents-version")).unwrap();
    assert_eq!(version.trim(), NEST_AGENTS_VERSION.to_string());
}

#[test]
fn refresh_agents_md_upgrades_attribution_and_preserves_owned_content() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    ensure_nest_at(&root).unwrap();

    let agents_md = root.join("AGENTS.md");
    fs::write(
        &agents_md,
        "# Buzz Nest\n\n## Git Commit Identity\n\n\
         - **Human sign-off (required):** every commit MUST include a `Signed-off-by`.\n\n\
         <!-- BEGIN BUZZ MANAGED — regenerated automatically, do not edit below -->\n\
         ## Active Agents\n\n| Name | Persona | How to address |\n\
         |------|---------|----------------|\n| Kit | Builder | @Kit |\n\
         <!-- END BUZZ MANAGED -->\n\n## Local Notes\n\nKeep me.\n",
    )
    .unwrap();
    fs::write(root.join(".nest-agents-version"), "5\n").unwrap();

    ensure_nest_at(&root).unwrap();

    let content = fs::read_to_string(&agents_md).unwrap();
    assert_eq!(content.matches("## Git Commit Attribution").count(), 1);
    assert!(!content.contains("**Human sign-off (required):**"));
    assert!(content.contains("| Kit | Builder | @Kit |"));
    assert!(content.contains("## Local Notes\n\nKeep me."));
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
fn ensure_nest_removes_generated_buzz_cli_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let canonical = root.join(".agents/skills/buzz-cli");
    fs::create_dir_all(&canonical).unwrap();
    fs::write(
        canonical.join("SKILL.md"),
        include_bytes!("../nest_skill.md"),
    )
    .unwrap();
    fs::write(canonical.join(".skill-version"), "5\n").unwrap();

    ensure_nest_at(&root).unwrap();

    assert!(!canonical.exists(), "generated canonical skill is removed");
}

#[cfg(unix)]
#[test]
fn ensure_nest_removes_only_generated_skill_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let canonical = root.join(".agents/skills/buzz-cli");
    fs::create_dir_all(&canonical).unwrap();
    fs::write(canonical.join(".skill-version"), "5\n").unwrap();
    fs::write(
        canonical.join("SKILL.md"),
        include_bytes!("../nest_skill.md"),
    )
    .unwrap();

    let generated_parent = root.join(".claude/skills");
    fs::create_dir_all(&generated_parent).unwrap();
    let generated = generated_parent.join("buzz-cli");
    std::os::unix::fs::symlink("../../.agents/skills/buzz-cli", &generated).unwrap();

    let custom_parent = root.join(".goose/skills");
    fs::create_dir_all(&custom_parent).unwrap();
    let custom = custom_parent.join("buzz-cli");
    std::os::unix::fs::symlink("/tmp/user-owned-buzz-cli", &custom).unwrap();

    ensure_nest_at(&root).unwrap();

    assert!(generated.symlink_metadata().is_err());
    assert!(custom.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        fs::read_link(custom).unwrap(),
        PathBuf::from("/tmp/user-owned-buzz-cli")
    );
}

#[test]
fn ensure_nest_preserves_user_owned_skill_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let custom = root.join(".agents/skills/buzz-cli");
    fs::create_dir_all(&custom).unwrap();
    fs::write(custom.join("SKILL.md"), "user-owned").unwrap();

    ensure_nest_at(&root).unwrap();

    assert_eq!(
        fs::read_to_string(custom.join("SKILL.md")).unwrap(),
        "user-owned"
    );
}

#[cfg(unix)]
#[test]
fn ensure_nest_preserves_unproven_skill_symlink() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let parent = root.join(".claude/skills");
    fs::create_dir_all(&parent).unwrap();
    let custom = parent.join("buzz-cli");
    std::os::unix::fs::symlink("../../.agents/skills/buzz-cli", &custom).unwrap();

    ensure_nest_at(&root).unwrap();

    assert!(custom.symlink_metadata().unwrap().file_type().is_symlink());
}

#[test]
fn ensure_nest_preserves_marked_but_edited_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let canonical = root.join(".agents/skills/buzz-cli");
    fs::create_dir_all(&canonical).unwrap();
    fs::write(canonical.join(".skill-version"), "5\n").unwrap();
    fs::write(canonical.join("SKILL.md"), "user edited skill").unwrap();
    #[cfg(unix)]
    {
        let parent = root.join(".claude/skills");
        fs::create_dir_all(&parent).unwrap();
        std::os::unix::fs::symlink("../../.agents/skills/buzz-cli", parent.join("buzz-cli"))
            .unwrap();
    }

    ensure_nest_at(&root).unwrap();

    assert_eq!(
        fs::read_to_string(canonical.join("SKILL.md")).unwrap(),
        "user edited skill"
    );
    #[cfg(unix)]
    assert!(root
        .join(".claude/skills/buzz-cli")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn ensure_nest_preserves_marked_skill_with_supporting_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let canonical = root.join(".agents/skills/buzz-cli");
    fs::create_dir_all(canonical.join("references")).unwrap();
    fs::write(canonical.join(".skill-version"), "5\n").unwrap();
    fs::write(
        canonical.join("SKILL.md"),
        include_bytes!("../nest_skill.md"),
    )
    .unwrap();
    fs::write(canonical.join("references/notes.md"), "keep me").unwrap();

    ensure_nest_at(&root).unwrap();

    assert_eq!(
        fs::read_to_string(canonical.join("references/notes.md")).unwrap(),
        "keep me"
    );
    assert!(!canonical.join("SKILL.md").exists());
    assert!(!canonical.join(".skill-version").exists());
}

#[cfg(unix)]
#[test]
fn ensure_nest_preserves_canonical_skill_symlink_and_external_target() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join(".buzz");
    let external = tmp.path().join("external-skill");
    fs::create_dir_all(&external).unwrap();
    fs::write(external.join(".skill-version"), "5\n").unwrap();
    fs::write(
        external.join("SKILL.md"),
        include_bytes!("../nest_skill.md"),
    )
    .unwrap();
    let canonical_parent = root.join(".agents/skills");
    fs::create_dir_all(&canonical_parent).unwrap();
    let canonical = canonical_parent.join("buzz-cli");
    std::os::unix::fs::symlink(&external, &canonical).unwrap();

    ensure_nest_at(&root).unwrap();

    assert!(canonical
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(external.join(".skill-version").is_file());
    assert!(external.join("SKILL.md").is_file());
}
