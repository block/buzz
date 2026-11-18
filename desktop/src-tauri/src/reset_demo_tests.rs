use super::*;

#[test]
fn test_demo_reset_preserves_shared_and_other_build_state() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    let app_data = tmp
        .path()
        .join("Application Support")
        .join("xyz.block.buzz.app.demo.current-1234567812345678");
    let demo_nest = home.join(".buzz-demo-current-1234567812345678");
    let prod_nest = home.join(".buzz");
    let other_demo_nest = home.join(".buzz-demo-other-8765432187654321");
    let shared_sprout = home.join(".sprout");
    let shared_agent = home.join(".config").join("buzz-agent");
    let demo_config = home
        .join("Library")
        .join("Application Support")
        .join("buzz-demo-current-1234567812345678");
    let demo_oauth = demo_config.join("buzz-agent").join("oauth");
    let other_demo_config = home
        .join("Library")
        .join("Application Support")
        .join("buzz-demo-other-8765432187654321");
    let other_demo_oauth = other_demo_config.join("buzz-agent").join("oauth");

    for path in [
        &app_data,
        &demo_nest,
        &prod_nest,
        &other_demo_nest,
        &shared_sprout,
        &shared_agent,
        &demo_oauth,
        &other_demo_oauth,
    ] {
        std::fs::create_dir_all(path).unwrap();
    }
    write_sentinel(&app_data).unwrap();

    let kc = FakeKeychain::ok();
    let ctx = ResetContext {
        app_data_dir: &app_data,
        legacy_app_data_dir: None,
        nest_dir: Some(demo_nest.clone()),
        keychain: &kc,
        home_dir: Some(home),
        is_dev: false,
        demo_config_dir: Some(demo_config.clone()),
        is_demo: true,
    };

    let outcome = run_boot_reset_with_keychain(ctx);

    assert!(outcome.completed, "demo reset must complete");
    assert!(!app_data.exists(), "demo app data must be wiped");
    assert!(!demo_nest.exists(), "selected demo nest must be wiped");
    assert!(
        !demo_config.exists(),
        "selected demo auth root must be wiped"
    );
    assert!(
        other_demo_oauth.exists(),
        "another demo's concrete auth root must survive"
    );
    assert!(prod_nest.exists(), "production nest must survive");
    assert!(other_demo_nest.exists(), "another demo nest must survive");
    assert!(shared_sprout.exists(), "shared legacy state must survive");
    assert!(
        shared_agent.exists(),
        "shared agent auth state must survive"
    );
}

#[test]
fn demo_config_delete_failure_keeps_sentinel_until_retry() {
    let tmp = TempDir::new().unwrap();
    let app_data = make_app_data(&tmp);
    let config = tmp.path().join("demo-config");
    let production = tmp.path().join("production/oauth/token.json");
    let sibling = tmp.path().join("sibling/oauth/token.json");
    for path in [&production, &sibling] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "preserve").unwrap();
    }
    // A file at the directory path makes remove_dir_all fail on every platform,
    // independent of the test user's privileges.
    std::fs::write(&config, "obstruction").unwrap();
    write_sentinel(&app_data).unwrap();
    let kc = FakeKeychain::ok();
    let run = || {
        let mut ctx = make_ctx(&app_data, &kc, false);
        ctx.is_demo = true;
        ctx.demo_config_dir = Some(config.clone());
        run_boot_reset_with_keychain(ctx)
    };
    let first = run();
    assert!(first.failed && !first.completed);
    assert!(check_sentinel(&app_data));
    assert!(config.exists());

    std::fs::remove_file(&config).unwrap();
    let token = config.join("buzz-agent/oauth/databricks/token.json");
    std::fs::create_dir_all(token.parent().unwrap()).unwrap();
    std::fs::write(&token, "demo credential").unwrap();
    let second = run();
    assert!(second.completed && !second.failed);
    assert!(!check_sentinel(&app_data));
    assert!(!config.exists());
    for path in [&production, &sibling] {
        assert_eq!(std::fs::read_to_string(path).unwrap(), "preserve");
    }
    // A retry after a crash that already removed the root must also succeed.
    write_sentinel(&app_data).unwrap();
    assert!(run().completed);
    assert!(!check_sentinel(&app_data));
}

#[cfg(unix)]
#[test]
fn demo_oauth_permission_failure_preserves_retry_intent() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().unwrap();
    let app_data = make_app_data(&tmp);
    let config = tmp.path().join("demo-config");
    let oauth = config.join("buzz-agent/oauth/databricks");
    let token = oauth.join("token.json");
    std::fs::create_dir_all(&oauth).unwrap();
    std::fs::write(&token, "demo credential").unwrap();
    write_sentinel(&app_data).unwrap();
    let kc = FakeKeychain::ok();
    let run = || {
        let mut ctx = make_ctx(&app_data, &kc, false);
        ctx.is_demo = true;
        ctx.demo_config_dir = Some(config.clone());
        run_boot_reset_with_keychain(ctx)
    };
    std::fs::set_permissions(&oauth, std::fs::Permissions::from_mode(0o500)).unwrap();
    let first = run();
    // Restore permissions before assertions so a failure never leaves test debris.
    if oauth.exists() {
        std::fs::set_permissions(&oauth, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    assert!(first.failed && !first.completed);
    assert!(check_sentinel(&app_data));
    assert_eq!(std::fs::read_to_string(&token).unwrap(), "demo credential");
    assert!(run().completed);
    assert!(!config.exists());
    assert!(!check_sentinel(&app_data));
}

#[test]
fn unresolved_demo_config_keeps_reset_pending_without_deleting_state() {
    let tmp = TempDir::new().unwrap();
    let app_data = make_app_data(&tmp);
    write_sentinel(&app_data).unwrap();
    let kc = FakeKeychain::ok();
    let mut ctx = make_ctx(&app_data, &kc, false);
    ctx.is_demo = true;
    assert!(ctx.demo_config_dir.is_none());
    let outcome = run_boot_reset_with_keychain(ctx);
    assert!(outcome.failed && !outcome.completed);
    assert!(check_sentinel(&app_data));
    assert!(
        app_data.exists(),
        "unresolved root must refuse before wiping"
    );
}
