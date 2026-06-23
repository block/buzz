use super::test_support::*;
use super::*;

#[test]
fn canonical_dev_data_dir_replaces_last_component() {
    let current =
        PathBuf::from("/Users/me/Library/Application Support/xyz.block.buzz.app.dev.my-branch");
    let canonical = canonical_dev_data_dir(&current).unwrap();
    assert_eq!(
        canonical,
        PathBuf::from("/Users/me/Library/Application Support/xyz.block.buzz.app.dev")
    );
}

#[test]
fn canonical_dev_data_dir_returns_none_for_root() {
    // A root path has no parent — should return None.
    assert!(canonical_dev_data_dir(Path::new("/")).is_none());
}

#[test]
fn legacy_app_data_dir_maps_release_identifier() {
    let current = PathBuf::from("/Users/me/Library/Application Support/xyz.block.buzz.app");
    let legacy = legacy_app_data_dir(&current).unwrap();
    assert_eq!(
        legacy,
        PathBuf::from("/Users/me/Library/Application Support/xyz.block.sprout.app")
    );
}

#[test]
fn legacy_app_data_dir_maps_dev_worktree_identifier() {
    let current =
        PathBuf::from("/Users/me/Library/Application Support/xyz.block.buzz.app.dev.my-branch");
    let legacy = legacy_app_data_dir(&current).unwrap();
    assert_eq!(
        legacy,
        PathBuf::from("/Users/me/Library/Application Support/xyz.block.sprout.app.dev.my-branch",)
    );
}

#[test]
fn copy_dir_all_preserves_nested_files_without_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("old");
    let dst = dir.path().join("new");
    std::fs::create_dir_all(src.join("agents")).unwrap();
    std::fs::write(src.join("identity.key"), "old-key").unwrap();
    std::fs::write(src.join("agents/managed-agents.json"), "old-agents").unwrap();
    std::fs::create_dir_all(&dst).unwrap();
    std::fs::write(dst.join("identity.key"), "new-key").unwrap();

    copy_dir_all(&src, &dst).unwrap();

    assert_eq!(
        std::fs::read_to_string(dst.join("identity.key")).unwrap(),
        "new-key"
    );
    assert_eq!(
        std::fs::read_to_string(dst.join("agents/managed-agents.json")).unwrap(),
        "old-agents"
    );
}

/// Helper: create a temp dir structure mimicking canonical + worktree layout.
/// Packs live in a `.main` sibling (not canonical) to match real-world state.
/// Returns `(parent_dir_handle, canonical_dir, worktree_dir)`.
#[cfg(unix)]
fn setup_sync_layout() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    let worktree = parent.path().join("xyz.block.buzz.app.dev.my-branch");
    let main_instance = parent.path().join("xyz.block.buzz.app.dev.main");

    std::fs::create_dir_all(canonical.join("agents")).unwrap();
    std::fs::write(
        canonical.join("agents/managed-agents.json"),
        r#"[{"id":"agent-1"}]"#,
    )
    .unwrap();
    std::fs::write(
        canonical.join("agents/personas.json"),
        r#"[{"id":"builtin:fizz"}]"#,
    )
    .unwrap();
    std::fs::write(canonical.join("agents/teams.json"), r#"[{"id":"team-1"}]"#).unwrap();

    // Teams installed from `.main` — canonical has no teams dir.
    let team_dir = main_instance.join("agents/teams/com.example.test-pack");
    std::fs::create_dir_all(&team_dir).unwrap();
    std::fs::write(team_dir.join("instructions.md"), "# Test pack").unwrap();
    std::fs::write(team_dir.join("fizz.persona.md"), "# Fizz").unwrap();

    (parent, canonical, worktree)
}

/// Helper: sync files directly (without a Tauri AppHandle) for unit testing.
/// Mirrors the symlink loop of `sync_shared_agent_data` but takes explicit
/// paths. `sync_shared_agent_data` requires a live Tauri AppHandle and
/// cannot be unit-tested directly.
#[cfg(unix)]
fn sync_files(canonical: &Path, worktree: &Path) -> u32 {
    let mut synced = 0u32;
    for rel in SHARED_AGENT_FILES {
        let src = canonical.join(rel);
        let dst = worktree.join(rel);
        if !src.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        if dst.is_symlink() {
            if let Ok(target) = std::fs::read_link(&dst) {
                if target == src {
                    continue;
                }
            }
        }
        if dst.exists() || dst.is_symlink() {
            let _ = std::fs::remove_file(&dst);
        }
        std::os::unix::fs::symlink(&src, &dst).unwrap();
        synced += 1;
    }
    // Migrate packs from siblings to canonical (mirrors production logic).
    for rel in SHARED_AGENT_DIRS {
        let canonical_target = canonical.join(rel);
        if !canonical_target.exists() {
            std::fs::create_dir_all(&canonical_target).unwrap();
            if let Some(parent) = canonical.parent() {
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let sibling = entry.path();
                        if sibling == canonical {
                            continue;
                        }
                        let sibling_dir = sibling.join(rel);
                        if sibling_dir.is_dir() && !sibling_dir.is_symlink() {
                            if let Ok(children) = std::fs::read_dir(&sibling_dir) {
                                for child in children.flatten() {
                                    let dest = canonical_target.join(child.file_name());
                                    if !dest.exists() {
                                        let _ = std::fs::rename(child.path(), &dest);
                                    }
                                }
                            }
                            let _ = std::fs::remove_dir_all(&sibling_dir);
                            let _ = std::os::unix::fs::symlink(&canonical_target, &sibling_dir);
                            break;
                        }
                    }
                }
            }
        }
    }

    for rel in SHARED_AGENT_DIRS {
        let src = canonical.join(rel);
        let dst = worktree.join(rel);
        if !src.exists() {
            continue;
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        if dst.is_symlink() {
            if let Ok(target) = std::fs::read_link(&dst) {
                if target == src {
                    continue;
                }
            }
        }
        if dst.is_symlink() {
            let _ = std::fs::remove_file(&dst);
        } else if dst.exists() {
            let _ = std::fs::remove_dir_all(&dst);
        }
        std::os::unix::fs::symlink(&src, &dst).unwrap();
        synced += 1;
    }
    synced
}

#[cfg(unix)]
#[test]
fn sync_creates_symlinks_to_fresh_worktree() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let synced = sync_files(&canonical, &worktree);
    assert_eq!(synced, 4);
    for rel in SHARED_AGENT_FILES {
        let dst = worktree.join(rel);
        assert!(dst.is_symlink(), "{rel} should be a symlink");
        assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
    }
    for rel in SHARED_AGENT_DIRS {
        let dst = worktree.join(rel);
        assert!(dst.is_symlink(), "{rel} should be a symlink");
        assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
    }
    assert_eq!(
        std::fs::read_to_string(worktree.join("agents/managed-agents.json")).unwrap(),
        r#"[{"id":"agent-1"}]"#,
    );
}

#[cfg(unix)]
#[test]
fn sync_replaces_existing_files_with_symlinks() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    std::fs::create_dir_all(worktree.join("agents")).unwrap();
    std::fs::write(worktree.join("agents/managed-agents.json"), "[]").unwrap();
    std::fs::write(worktree.join("agents/personas.json"), "[]").unwrap();
    std::fs::write(worktree.join("agents/teams.json"), "[]").unwrap();

    let synced = sync_files(&canonical, &worktree);

    assert_eq!(synced, 4);
    for rel in SHARED_AGENT_FILES {
        let dst = worktree.join(rel);
        assert!(
            dst.is_symlink(),
            "{rel} should be a symlink after replacing regular file"
        );
        assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
    }
    assert_eq!(
        std::fs::read_to_string(worktree.join("agents/managed-agents.json")).unwrap(),
        r#"[{"id":"agent-1"}]"#,
    );
}

#[cfg(unix)]
#[test]
fn sync_preserves_correct_symlinks() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    assert_eq!(sync_files(&canonical, &worktree), 4);
    assert_eq!(sync_files(&canonical, &worktree), 0);
    for rel in SHARED_AGENT_FILES {
        let dst = worktree.join(rel);
        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
    }
}

#[cfg(unix)]
#[test]
fn sync_replaces_wrong_symlinks() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let wrong_target = PathBuf::from("/nonexistent/wrong-target.json");
    std::fs::create_dir_all(worktree.join("agents")).unwrap();
    for rel in SHARED_AGENT_FILES {
        std::os::unix::fs::symlink(&wrong_target, worktree.join(rel)).unwrap();
    }
    let synced = sync_files(&canonical, &worktree);
    assert_eq!(synced, 4);
    for rel in SHARED_AGENT_FILES {
        assert_eq!(
            std::fs::read_link(worktree.join(rel)).unwrap(),
            canonical.join(rel)
        );
    }
}

#[cfg(unix)]
#[test]
fn sync_handles_broken_symlinks() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    std::fs::create_dir_all(worktree.join("agents")).unwrap();
    let broken_target = PathBuf::from("/this/does/not/exist.json");
    for rel in SHARED_AGENT_FILES {
        std::os::unix::fs::symlink(&broken_target, worktree.join(rel)).unwrap();
    }
    let synced = sync_files(&canonical, &worktree);
    assert_eq!(synced, 4);
    for rel in SHARED_AGENT_FILES {
        let dst = worktree.join(rel);
        assert!(dst.is_symlink());
        assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
        // Content should be readable through the fixed symlink.
        assert!(std::fs::read_to_string(&dst).is_ok());
    }
}

#[cfg(unix)]
#[test]
fn writes_through_symlink_reach_canonical() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    sync_files(&canonical, &worktree);

    let worktree_path = worktree.join("agents/personas.json");
    let canonical_path = canonical.join("agents/personas.json");

    // Write through the symlink using the same pattern as atomic_write_json.
    let new_content = r#"[{"id":"builtin:fizz","updated":true}]"#;
    let resolved = std::fs::canonicalize(&worktree_path).unwrap();
    let tmp = resolved.with_extension("json.tmp");
    std::fs::write(&tmp, new_content.as_bytes()).unwrap();
    std::fs::rename(&tmp, &resolved).unwrap();

    // The canonical file should have the new content.
    assert_eq!(
        std::fs::read_to_string(&canonical_path).unwrap(),
        new_content
    );
    // The worktree path should still be a symlink.
    assert!(worktree_path.is_symlink());
    // Reading through the symlink should return the new content.
    assert_eq!(
        std::fs::read_to_string(&worktree_path).unwrap(),
        new_content
    );
}

#[test]
fn canonical_dev_data_dir_returns_self_for_canonical_instance() {
    // When the current app data dir IS the canonical dev identifier,
    // canonical_dev_data_dir returns the exact same path — the caller
    // (sync_shared_agent_data) uses this equality to skip the sync.
    // The env-var guards (BUZZ_SHARE_IDENTITY, BUZZ_PRIVATE_KEY)
    // require a live Tauri AppHandle and are covered by integration
    // testing only.
    let current = PathBuf::from("/Users/me/Library/Application Support/xyz.block.buzz.app.dev");
    assert_eq!(canonical_dev_data_dir(&current).unwrap(), current);

    // Also verify with a temp dir on the real filesystem.
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    assert_eq!(canonical_dev_data_dir(&canonical).unwrap(), canonical);
}

#[cfg(unix)]
#[test]
fn sync_creates_teams_directory_symlink() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    sync_files(&canonical, &worktree);

    let teams_link = worktree.join("agents/teams");
    assert!(teams_link.is_symlink());
    assert_eq!(
        std::fs::read_link(&teams_link).unwrap(),
        canonical.join("agents/teams")
    );
    assert_eq!(
        std::fs::read_to_string(
            worktree.join("agents/teams/com.example.test-pack/instructions.md")
        )
        .unwrap(),
        "# Test pack"
    );
}

#[cfg(unix)]
#[test]
fn sync_migrates_teams_from_sibling_to_canonical() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let main_instance = canonical
        .parent()
        .unwrap()
        .join("xyz.block.buzz.app.dev.main");

    // Before sync: canonical has no teams, .main has the real team dir.
    assert!(!canonical.join("agents/teams").exists());
    assert!(main_instance
        .join("agents/teams/com.example.test-pack")
        .is_dir());

    sync_files(&canonical, &worktree);

    // After sync: canonical has the team, .main is now a symlink.
    assert!(canonical
        .join("agents/teams/com.example.test-pack/instructions.md")
        .exists());
    assert!(main_instance.join("agents/teams").is_symlink());
    assert_eq!(
        std::fs::read_link(main_instance.join("agents/teams")).unwrap(),
        canonical.join("agents/teams")
    );
}

#[cfg(unix)]
#[test]
fn sync_replaces_real_teams_dir_with_symlink() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let real_teams = worktree.join("agents/teams");
    std::fs::create_dir_all(&real_teams).unwrap();
    std::fs::write(real_teams.join("stale-file.txt"), "stale").unwrap();

    sync_files(&canonical, &worktree);

    assert!(worktree.join("agents/teams").is_symlink());
    assert_eq!(
        std::fs::read_link(worktree.join("agents/teams")).unwrap(),
        canonical.join("agents/teams")
    );
}

// ── Packs → Teams migration tests ───────────────────────────────────

#[cfg(unix)]
#[test]
fn migrate_packs_merge_preserves_non_empty_dir() {
    // When packs/ contains symlinks that weren't moved (e.g., external tools
    // recreated them), the migration should NOT delete the packs/ directory.
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    let packs_dir = canonical.join("agents/packs");
    let teams_dir = canonical.join("agents/teams");
    std::fs::create_dir_all(&packs_dir).unwrap();
    std::fs::create_dir_all(&teams_dir).unwrap();

    // Simulate an external symlink that already exists in teams/ (conflict)
    let external_target = parent.path().join("external-pack");
    std::fs::create_dir_all(&external_target).unwrap();
    std::os::unix::fs::symlink(&external_target, packs_dir.join("com.ext.pack")).unwrap();
    // Same name already in teams/ — so the migration skips it
    std::os::unix::fs::symlink(&external_target, teams_dir.join("com.ext.pack")).unwrap();

    // Run the merge logic (mirrors what migrate_packs_to_teams does)
    if let Ok(entries) = std::fs::read_dir(&packs_dir) {
        for entry in entries.flatten() {
            let dest = teams_dir.join(entry.file_name());
            if !dest.exists() {
                let _ = std::fs::rename(entry.path(), &dest);
            }
        }
    }
    // This is the fix: remove_dir only succeeds on empty dirs
    let _ = std::fs::remove_dir(&packs_dir);

    // packs/ should still exist because it has a remaining symlink
    assert!(packs_dir.exists(), "packs/ should survive when non-empty");
    assert!(packs_dir.join("com.ext.pack").is_symlink());
}

#[test]
fn migrate_packs_to_teams_renames_directory() {
    let parent = tempfile::tempdir().unwrap();
    let canonical = parent.path().join(CANONICAL_DEV_IDENTIFIER);
    let packs_dir = canonical.join("agents/packs/com.example.test-pack");
    std::fs::create_dir_all(&packs_dir).unwrap();
    std::fs::write(packs_dir.join("plugin.json"), "{}").unwrap();

    // No personas or agents JSON needed for directory rename
    std::fs::create_dir_all(canonical.join("agents")).unwrap();

    // Simulate calling the migration steps directly (no AppHandle needed)
    let packs = canonical.join("agents/packs");
    let teams = canonical.join("agents/teams");
    std::fs::rename(&packs, &teams).unwrap();

    assert!(!packs.exists());
    assert!(teams.join("com.example.test-pack/plugin.json").exists());
}

#[test]
fn migrate_packs_to_teams_rewrites_personas_json() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "display_name": "Test",
            "source_pack": "com.example.my-pack",
            "source_pack_persona_slug": "agent-one"
        }]),
    );

    let path = dir.path().join("agents/personas.json");
    patch_json_records(&path, |obj| {
        let mut changed = false;
        if let Some(val) = obj.remove("source_pack") {
            obj.insert("source_team".to_string(), val);
            changed = true;
        }
        if let Some(val) = obj.remove("source_pack_persona_slug") {
            obj.insert("source_team_persona_slug".to_string(), val);
            changed = true;
        }
        changed
    });

    let records = read_personas_json(dir.path());
    assert_eq!(records[0]["source_team"], "com.example.my-pack");
    assert_eq!(records[0]["source_team_persona_slug"], "agent-one");
    assert!(records[0].get("source_pack").is_none());
    assert!(records[0].get("source_pack_persona_slug").is_none());
}

#[test]
fn migrate_packs_to_teams_rewrites_agents_json() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Paul",
            "persona_pack_path": "/data/agents/packs/com.example.my-pack",
            "persona_name_in_pack": "agent-one"
        }]),
    );

    let path = dir.path().join("agents/managed-agents.json");
    patch_json_records(&path, |obj| {
        let mut changed = false;
        if let Some(val) = obj.remove("persona_pack_path") {
            let new_val = if let Some(s) = val.as_str() {
                serde_json::Value::String(s.replace("/packs/", "/teams/"))
            } else {
                val
            };
            obj.insert("persona_team_dir".to_string(), new_val);
            changed = true;
        }
        if let Some(val) = obj.remove("persona_name_in_pack") {
            obj.insert("persona_name_in_team".to_string(), val);
            changed = true;
        }
        changed
    });

    let records = read_agents_json(dir.path());
    assert_eq!(
        records[0]["persona_team_dir"],
        "/data/agents/teams/com.example.my-pack"
    );
    assert_eq!(records[0]["persona_name_in_team"], "agent-one");
    assert!(records[0].get("persona_pack_path").is_none());
    assert!(records[0].get("persona_name_in_pack").is_none());
}

#[test]
fn rename_provider_to_runtime_migrates_field() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "displayName": "Alice",
            "provider": "goose"
        }]),
    );
    rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
    let records = read_personas_json(dir.path());
    assert_eq!(records[0]["runtime"], "goose");
    assert!(records[0].get("provider").is_none());
}

#[test]
fn rename_provider_to_runtime_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "displayName": "Alice",
            "runtime": "goose"
        }]),
    );
    let before = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
    rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
    let after = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
    assert_eq!(
        before, after,
        "file should not be rewritten when already migrated"
    );
}

#[test]
fn rename_provider_to_runtime_skips_record_without_either_key() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "displayName": "Alice"
        }]),
    );
    let before = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
    rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
    let after = std::fs::read_to_string(dir.path().join("agents/personas.json")).unwrap();
    assert_eq!(
        before, after,
        "file should not be rewritten when no provider key exists"
    );
}

#[test]
fn rename_provider_to_runtime_preserves_existing_runtime_over_provider() {
    let dir = tempfile::tempdir().unwrap();
    write_personas_json(
        dir.path(),
        &serde_json::json!([{
            "id": "persona-1",
            "displayName": "Alice",
            "provider": "old-value",
            "runtime": "correct-value"
        }]),
    );
    rename_provider_to_runtime_in_personas(&dir.path().join("agents/personas.json"));
    let records = read_personas_json(dir.path());
    assert_eq!(records[0]["runtime"], "correct-value");
    // provider key should still be there since the closure returns false when runtime exists
    assert_eq!(records[0]["provider"], "old-value");
}

#[test]
fn reconcile_mcp_commands_clears_stale_buzz_mcp_server() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Fizz",
            "agent_command": "goose",
            "mcp_command": "buzz-mcp-server"
        }]),
    );
    reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["mcp_command"], "");
}

#[test]
fn reconcile_mcp_commands_sets_canonical_for_buzz_agent() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Stilgar",
            "agent_command": "buzz-agent",
            "mcp_command": "buzz-mcp-server"
        }]),
    );
    reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["mcp_command"], "buzz-dev-mcp");
}

#[test]
fn reconcile_mcp_commands_leaves_custom_value_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let json = serde_json::json!([{
        "name": "Fizz",
        "agent_command": "goose",
        "mcp_command": "my-custom-mcp"
    }]);
    write_agents_json(dir.path(), &json);
    let path = dir.path().join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();
    reconcile_mcp_commands_in_file(&path);
    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

#[test]
fn reconcile_mcp_commands_leaves_unknown_runtime_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let json = serde_json::json!([{
        "name": "Custom",
        "agent_command": "my-custom-agent",
        "mcp_command": "buzz-mcp-server"
    }]);
    write_agents_json(dir.path(), &json);
    let path = dir.path().join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();
    reconcile_mcp_commands_in_file(&path);
    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

#[test]
fn reconcile_mcp_commands_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Fizz",
            "agent_command": "goose",
            "mcp_command": "buzz-mcp-server"
        }]),
    );
    let path = dir.path().join("agents/managed-agents.json");
    reconcile_mcp_commands_in_file(&path);
    let after_first = std::fs::read_to_string(&path).unwrap();
    reconcile_mcp_commands_in_file(&path);
    assert_eq!(after_first, std::fs::read_to_string(&path).unwrap());
}

#[test]
fn reconcile_mcp_commands_handles_mixed_agents() {
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([
            {"name": "Stale Goose", "agent_command": "goose", "mcp_command": "buzz-mcp-server"},
            {"name": "Clean Goose", "agent_command": "goose", "mcp_command": ""},
            {"name": "Custom Agent", "agent_command": "goose", "mcp_command": "my-custom-mcp"},
            {"name": "Stale Buzz", "agent_command": "buzz-agent", "mcp_command": "buzz-mcp-server"}
        ]),
    );
    reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["mcp_command"], "");
    assert_eq!(records[1]["mcp_command"], "");
    assert_eq!(records[2]["mcp_command"], "my-custom-mcp");
    assert_eq!(records[3]["mcp_command"], "buzz-dev-mcp");
}

#[test]
fn reconcile_mcp_commands_skips_record_without_agent_command() {
    let dir = tempfile::tempdir().unwrap();
    let json = serde_json::json!([{
        "name": "No Command",
        "mcp_command": "buzz-mcp-server"
    }]);
    write_agents_json(dir.path(), &json);
    let path = dir.path().join("agents/managed-agents.json");
    let before = std::fs::read_to_string(&path).unwrap();
    reconcile_mcp_commands_in_file(&path);
    assert_eq!(before, std::fs::read_to_string(&path).unwrap());
}

/// Helper: write a `personas.json` directly in `base_dir` (the migration
/// reads `base_dir/personas.json`, where `base_dir` is the `agents` dir).
fn write_base_personas(base_dir: &Path, records: &serde_json::Value) {
    std::fs::write(
        base_dir.join("personas.json"),
        serde_json::to_string_pretty(records).unwrap(),
    )
    .unwrap();
}

fn one_persona() -> serde_json::Value {
    serde_json::json!([{
        "id": "code-reviewer",
        "display_name": "Code Reviewer",
        "system_prompt": "You review code.",
        "is_builtin": false,
        "is_active": true,
        "name_pool": [],
        "env_vars": {},
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    }])
}

#[test]
fn migrate_personas_writes_signed_retention_rows() {
    use crate::managed_agents::retention::{get_retained_personas, open_retention_db};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    let migrated = migrate_personas_in_dir(base.path(), &keys).unwrap();
    assert_eq!(migrated, 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let rows = get_retained_personas(&conn, &pubkey).unwrap();
    assert_eq!(rows.len(), 1);
    // Row holds a real signed event for the owner — not a placeholder.
    assert_eq!(rows[0].pubkey, pubkey);
    let event: nostr::Event = nostr::JsonUtil::from_json(&rows[0].raw_event).unwrap();
    assert!(event.verify().is_ok());
    assert!(rows[0].pending_sync);
}

#[test]
fn migrate_personas_skips_builtins() {
    use crate::managed_agents::retention::{get_retained_personas, open_retention_db};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(
        base.path(),
        &serde_json::json!([{
            "id": "builtin:solo",
            "display_name": "Solo",
            "system_prompt": "x",
            "is_builtin": true,
            "is_active": true,
            "name_pool": [],
            "env_vars": {},
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z"
        }]),
    );
    let keys = nostr::Keys::generate();

    let migrated = migrate_personas_in_dir(base.path(), &keys).unwrap();
    assert_eq!(migrated, 0);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let rows = get_retained_personas(&conn, &keys.public_key().to_hex()).unwrap();
    assert!(rows.is_empty());
}

#[test]
fn migrate_personas_unchanged_second_run_is_noop() {
    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();

    // First run retains; second run with identical personas re-retains
    // nothing — the per-coordinate content matches, so `pending_sync` is
    // not churned.
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 0);
    assert!(!base.path().join("migration_state.json").exists());
}

#[test]
fn migrate_personas_new_persona_after_first_run_gets_retained() {
    use crate::managed_agents::retention::{get_retained_personas, open_retention_db};

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    // A persona added to personas.json after the first reconcile must be
    // picked up — the whole-store sentinel that previously short-circuited
    // this is gone.
    let mut two = one_persona();
    two.as_array_mut().unwrap().push(serde_json::json!({
        "id": "test-writer",
        "display_name": "Test Writer",
        "system_prompt": "You write tests.",
        "is_builtin": false,
        "is_active": true,
        "name_pool": [],
        "env_vars": {},
        "created_at": "2025-01-02T00:00:00Z",
        "updated_at": "2025-01-02T00:00:00Z"
    }));
    write_base_personas(base.path(), &two);

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let rows = get_retained_personas(&conn, &pubkey).unwrap();
    assert_eq!(rows.len(), 2);
}

#[test]
fn migrate_personas_edited_persona_re_retains_pending() {
    use crate::managed_agents::retention::{get_retained_event, mark_synced, open_retention_db};
    use buzz_core_pkg::kind::KIND_PERSONA;

    let base = tempfile::tempdir().unwrap();
    write_base_personas(base.path(), &one_persona());
    let keys = nostr::Keys::generate();
    let pubkey = keys.public_key().to_hex();

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    // Simulate the flush loop confirming the first publish.
    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
        .unwrap()
        .unwrap();
    mark_synced(
        &conn,
        KIND_PERSONA,
        &pubkey,
        "code-reviewer",
        row.created_at,
        &row.content,
    )
    .unwrap();
    drop(conn);

    // Editing the persona on disk must re-retain it as pending so the edit
    // reaches the relay on the next flush.
    let mut edited = one_persona();
    edited.as_array_mut().unwrap()[0]["system_prompt"] =
        serde_json::json!("You review code carefully.");
    write_base_personas(base.path(), &edited);

    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 1);

    let conn = open_retention_db(&base.path().join("retention.db")).unwrap();
    let row = get_retained_event(&conn, KIND_PERSONA, &pubkey, "code-reviewer")
        .unwrap()
        .unwrap();
    assert!(row.pending_sync);
    assert!(row.content.contains("carefully"));
}

#[test]
fn migrate_personas_no_file_is_noop() {
    let base = tempfile::tempdir().unwrap();
    let keys = nostr::Keys::generate();
    assert_eq!(migrate_personas_in_dir(base.path(), &keys).unwrap(), 0);
}
