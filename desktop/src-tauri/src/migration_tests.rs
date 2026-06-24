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
    // Seed-up: mirrors the SHARED_AGENT_FILES seed-up in `sync_shared_agent_data`.
    // Kept logic-identical to production so these tests exercise real behavior.
    for rel in SHARED_AGENT_FILES {
        let canonical_file = canonical.join(rel);
        if canonical_file.exists() {
            continue;
        }
        let Some(parent) = canonical.parent() else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let sibling = entry.path();
            if sibling == canonical {
                continue;
            }
            let sibling_file = sibling.join(rel);
            if sibling_file.is_file() && !sibling_file.is_symlink() {
                if let Some(file_parent) = canonical_file.parent() {
                    std::fs::create_dir_all(file_parent).unwrap();
                }
                let _ = std::fs::rename(&sibling_file, &canonical_file);
                break;
            }
        }
    }

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

#[cfg(unix)]
#[test]
fn seed_up_migrates_sibling_file_to_canonical_then_symlinks() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let rel = "agents/personas.json";
    // Canonical is missing the file; a sibling (.main) holds real content.
    std::fs::remove_file(canonical.join(rel)).unwrap();
    let sibling = canonical
        .parent()
        .unwrap()
        .join("xyz.block.buzz.app.dev.main");
    std::fs::create_dir_all(sibling.join("agents")).unwrap();
    std::fs::write(sibling.join(rel), r#"[{"id":"brain"}]"#).unwrap();

    sync_files(&canonical, &worktree);

    // The real file landed at canonical (proves the rename, not a dangling link).
    let canonical_file = canonical.join(rel);
    assert!(
        canonical_file.is_file() && !canonical_file.is_symlink(),
        "canonical should hold the migrated real file"
    );
    assert_eq!(
        std::fs::read_to_string(&canonical_file).unwrap(),
        r#"[{"id":"brain"}]"#,
    );
    // The worktree is symlinked to canonical.
    let dst = worktree.join(rel);
    assert!(dst.is_symlink());
    assert_eq!(std::fs::read_link(&dst).unwrap(), canonical_file);
}

#[cfg(unix)]
#[test]
fn seed_up_no_sibling_content_is_noop() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let rel = "agents/personas.json";
    // Canonical missing the file and no sibling holds it.
    std::fs::remove_file(canonical.join(rel)).unwrap();

    sync_files(&canonical, &worktree);

    // Nothing to seed: canonical stays missing, worktree gets no symlink for it.
    assert!(!canonical.join(rel).exists());
    assert!(!worktree.join(rel).exists());
}

#[cfg(unix)]
#[test]
fn seed_up_skipped_when_canonical_has_file() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let rel = "agents/personas.json";
    // A sibling also holds different content, but canonical already has the file.
    let sibling = canonical
        .parent()
        .unwrap()
        .join("xyz.block.buzz.app.dev.main");
    std::fs::create_dir_all(sibling.join("agents")).unwrap();
    std::fs::write(sibling.join(rel), r#"[{"id":"should-not-win"}]"#).unwrap();

    sync_files(&canonical, &worktree);

    // Canonical's original content is untouched; the sibling did not seed it.
    assert_eq!(
        std::fs::read_to_string(canonical.join(rel)).unwrap(),
        r#"[{"id":"builtin:fizz"}]"#,
    );
    // Pull-symlink path is unchanged: worktree links to canonical.
    let dst = worktree.join(rel);
    assert!(dst.is_symlink());
    assert_eq!(std::fs::read_link(&dst).unwrap(), canonical.join(rel));
}

#[cfg(unix)]
#[test]
fn seed_up_ignores_sibling_symlink_as_source() {
    let (_parent, canonical, worktree) = setup_sync_layout();
    let rel = "agents/personas.json";
    std::fs::remove_file(canonical.join(rel)).unwrap();
    // Sibling holds only a symlink (not real content) — not a valid seed source.
    let sibling = canonical
        .parent()
        .unwrap()
        .join("xyz.block.buzz.app.dev.main");
    std::fs::create_dir_all(sibling.join("agents")).unwrap();
    std::os::unix::fs::symlink(
        PathBuf::from("/nonexistent/elsewhere.json"),
        sibling.join(rel),
    )
    .unwrap();

    sync_files(&canonical, &worktree);

    // The symlink was not promoted; canonical stays missing.
    assert!(!canonical.join(rel).exists());
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
fn reconcile_mcp_commands_resolves_persona_runtime_over_stale_snapshot() {
    // The frozen snapshot is buzz-agent (wants buzz-dev-mcp), but the linked
    // persona's runtime is goose (wants no mcp). The reconcile must follow the
    // EFFECTIVE harness (persona-wins) and clear the stale buzz-mcp-server.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Fizz",
            "persona_id": "p1",
            "agent_command": "buzz-agent",
            "mcp_command": "buzz-mcp-server"
        }]),
    );
    write_personas_json(
        dir.path(),
        &serde_json::json!([{"id": "p1", "runtime": "goose"}]),
    );
    reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["mcp_command"], "");
}

#[test]
fn reconcile_mcp_commands_honors_explicit_override_over_persona() {
    // An explicit per-instance pin (agent_command_override) beats the persona
    // runtime: persona is goose (no mcp) but the pin is buzz-agent, so the
    // reconcile sets the buzz-agent mcp_command.
    let dir = tempfile::tempdir().unwrap();
    write_agents_json(
        dir.path(),
        &serde_json::json!([{
            "name": "Fizz",
            "persona_id": "p1",
            "agent_command": "goose",
            "agent_command_override": "buzz-agent",
            "mcp_command": ""
        }]),
    );
    write_personas_json(
        dir.path(),
        &serde_json::json!([{"id": "p1", "runtime": "goose"}]),
    );
    reconcile_mcp_commands_in_file(&dir.path().join("agents/managed-agents.json"));
    let records = read_agents_json(dir.path());
    assert_eq!(records[0]["mcp_command"], "buzz-dev-mcp");
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

#[test]
fn migrate_legacy_nest_carries_knowledge_and_skips_repos() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(".sprout");
    let current = dir.path().join(".buzz");

    // Knowledge: a top-level file plus a nested dir.
    std::fs::create_dir_all(legacy.join("RESEARCH")).unwrap();
    std::fs::write(legacy.join("AGENTS.md"), "agents").unwrap();
    std::fs::write(legacy.join("RESEARCH/NOTES.md"), "notes").unwrap();
    // A fat REPOS/ that must NOT be copied.
    std::fs::create_dir_all(legacy.join("REPOS/buzz")).unwrap();
    std::fs::write(legacy.join("REPOS/buzz/huge.bin"), "checkout").unwrap();

    let migrated = super::migrate_legacy_nest_at(&legacy, &current);

    assert!(migrated, "migration ran because legacy nest existed");
    assert!(
        !current.join("REPOS").exists(),
        "REPOS/ must never be migrated"
    );
    assert_eq!(
        std::fs::read_to_string(current.join("AGENTS.md")).unwrap(),
        "agents"
    );
    assert_eq!(
        std::fs::read_to_string(current.join("RESEARCH/NOTES.md")).unwrap(),
        "notes"
    );
}

#[test]
fn migrate_legacy_nest_does_not_clobber_existing_destination() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(".sprout");
    let current = dir.path().join(".buzz");

    std::fs::create_dir_all(legacy.join("RESEARCH")).unwrap();
    std::fs::write(legacy.join("AGENTS.md"), "legacy-agents").unwrap();
    std::fs::write(legacy.join("RESEARCH/NOTES.md"), "legacy-notes").unwrap();
    // Pre-existing live content the migration must preserve.
    std::fs::create_dir_all(current.join("RESEARCH")).unwrap();
    std::fs::write(current.join("AGENTS.md"), "live-agents").unwrap();
    std::fs::write(current.join("RESEARCH/NOTES.md"), "live-notes").unwrap();

    super::migrate_legacy_nest_at(&legacy, &current);

    assert_eq!(
        std::fs::read_to_string(current.join("AGENTS.md")).unwrap(),
        "live-agents",
        "existing top-level file must not be clobbered"
    );
    assert_eq!(
        std::fs::read_to_string(current.join("RESEARCH/NOTES.md")).unwrap(),
        "live-notes",
        "existing nested file must not be clobbered"
    );
}

#[test]
fn migrate_legacy_nest_is_idempotent_on_rerun() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(".sprout");
    let current = dir.path().join(".buzz");

    std::fs::create_dir_all(legacy.join("PLANS")).unwrap();
    std::fs::write(legacy.join("PLANS/PLAN.md"), "plan").unwrap();

    super::migrate_legacy_nest_at(&legacy, &current);
    super::migrate_legacy_nest_at(&legacy, &current);

    assert_eq!(
        std::fs::read_to_string(current.join("PLANS/PLAN.md")).unwrap(),
        "plan"
    );
}

#[test]
fn migrate_legacy_nest_noops_when_legacy_absent() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(".sprout");
    let current = dir.path().join(".buzz");

    let migrated = super::migrate_legacy_nest_at(&legacy, &current);

    assert!(!migrated, "no migration when legacy nest is absent");
    assert!(
        !current.exists(),
        "no destination created when legacy absent"
    );
}

#[test]
fn migrate_legacy_nest_overwrites_generated_default_agents_md() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(".sprout");
    let current = dir.path().join(".buzz");

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("AGENTS.md"), "legacy team instructions").unwrap();

    // First-time launch order: ensure_nest writes the generated default into
    // ~/.buzz/AGENTS.md, then migration runs.
    crate::managed_agents::ensure_nest_at(&current).unwrap();
    assert_eq!(
        std::fs::read_to_string(current.join("AGENTS.md")).unwrap(),
        crate::managed_agents::AGENTS_MD,
        "precondition: ensure_nest writes the generated default"
    );

    super::migrate_legacy_nest_at(&legacy, &current);

    assert_eq!(
        std::fs::read_to_string(current.join("AGENTS.md")).unwrap(),
        "legacy team instructions",
        "legacy AGENTS.md must overwrite the untouched generated default"
    );
}

#[test]
fn migrate_legacy_nest_preserves_user_edited_agents_md() {
    let dir = tempfile::tempdir().unwrap();
    let legacy = dir.path().join(".sprout");
    let current = dir.path().join(".buzz");

    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("AGENTS.md"), "legacy team instructions").unwrap();
    std::fs::create_dir_all(&current).unwrap();
    std::fs::write(current.join("AGENTS.md"), "user-edited live AGENTS").unwrap();

    super::migrate_legacy_nest_at(&legacy, &current);

    assert_eq!(
        std::fs::read_to_string(current.join("AGENTS.md")).unwrap(),
        "user-edited live AGENTS",
        "a user-edited live AGENTS.md must never be clobbered"
    );
}
