//! Runtime authority migration and production Start regressions.

use super::super as runtime;
use super::receipt_fixture;

#[test]
fn legacy_receipt_validation_uses_legacy_rendering_for_global_ownership() {
    let mut receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new(
            "aa".repeat(32),
            "wss://relay.example?mode=one",
        )
        .unwrap(),
    );
    receipt.authority_version = 0;
    // url::Url serialization retained the root slash before a query in V0,
    // while the scoped runtime renderer intentionally removes that root slash.
    receipt.key.relay_url = "wss://relay.example/?mode=one".into();
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));

    assert!(runtime::valid_agent_runtime_receipt_with(
        &path,
        &receipt,
        "test-instance",
        |_| true,
        |_, _| true,
    ));
}

#[test]
fn legacy_normalizer_loss_boundaries_are_pinned_to_the_real_url_renderer() {
    let normalize = buzz_core_pkg::relay::normalize_relay_url;
    assert_eq!(
        normalize("wss://relay.example/room/").unwrap(),
        "wss://relay.example/room"
    );
    assert_eq!(
        normalize("wss://relay.example/room?tail=/").unwrap(),
        "wss://relay.example/room?tail="
    );
    assert_eq!(
        normalize("wss://relay.example/?mode=one").unwrap(),
        "wss://relay.example/?mode=one"
    );
    assert_eq!(
        normalize("wss://relay.example/?").unwrap(),
        "wss://relay.example/?"
    );
}

#[test]
fn replacement_removes_receipt_only_after_confirmed_exit() {
    use std::cell::{Cell, RefCell};

    let receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
            .unwrap(),
    );
    let path = std::path::Path::new("pair.json");
    let terminated = Cell::new(None);
    let polls = Cell::new(0);
    let removed = RefCell::new(None);

    runtime::terminate_runtime_receipt_with(
        path,
        &receipt,
        |pid| {
            terminated.set(Some(pid));
            Ok(())
        },
        |_| {
            let poll = polls.get() + 1;
            polls.set(poll);
            poll < 2
        },
        |path| *removed.borrow_mut() = Some(path.to_path_buf()),
    )
    .unwrap();

    assert_eq!(terminated.get(), Some(receipt.pid));
    assert_eq!(polls.get(), 2);
    assert_eq!(removed.into_inner().as_deref(), Some(path));
}

#[test]
fn replacement_failure_keeps_receipt() {
    use std::cell::Cell;

    let receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
            .unwrap(),
    );
    let removed = Cell::new(false);
    let error = runtime::terminate_runtime_receipt_with(
        std::path::Path::new("pair.json"),
        &receipt,
        |_| Err("signal failed".into()),
        |_| false,
        |_| removed.set(true),
    )
    .unwrap_err();

    assert_eq!(error, "signal failed");
    assert!(!removed.get());
}

#[cfg(unix)]
#[test]
fn production_start_refuses_live_unversioned_receipt_before_spawn() {
    let _path_guard = crate::managed_agents::lock_path_mutex();
    let temp = tempfile::tempdir().unwrap();
    let old_home = std::env::var_os("HOME");
    let old_xdg = std::env::var_os("XDG_DATA_HOME");
    struct RestoreEnv(Option<std::ffi::OsString>, Option<std::ffi::OsString>);
    impl Drop for RestoreEnv {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
            match self.1.take() {
                Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
        }
    }
    let _restore_env = RestoreEnv(old_home, old_xdg);
    std::env::set_var("HOME", temp.path());
    std::env::set_var("XDG_DATA_HOME", temp.path());

    let relay = "wss://relay.example";
    let pubkey = "aa".repeat(32);
    let app = tauri::test::mock_builder()
        .manage(crate::app_state::build_app_state())
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let instance_id = runtime::current_instance_id(app.handle());
    let mut child = runtime::test_fixtures::MarkedTestChild::spawn(&instance_id).unwrap();
    assert!(runtime::process_has_buzz_marker(child.id(), &instance_id));

    let key = crate::managed_agents::ManagedAgentRuntimeKey::new(&pubkey, relay).unwrap();
    let receipt = crate::managed_agents::ManagedAgentRuntimeReceipt {
        authority_version: 0,
        key: key.clone(),
        pid: child.id(),
        desktop_instance_id: instance_id,
        started_at: "now".into(),
    };
    crate::managed_agents::write_agent_runtime_receipt(app.handle(), &receipt).unwrap();

    let mut record = runtime::test_fixtures::fixture(
        crate::managed_agents::RespondTo::OwnerOnly,
        Vec::new(),
        None,
    );
    record.pubkey = pubkey;
    record.acp_command = "a-command-that-must-not-be-resolved".into();
    let bound = crate::relay::bind_expected_relay_scope(None, relay.into()).unwrap();
    let mut runtimes = std::collections::HashMap::new();

    let error = runtime::start_managed_agent_process(
        app.handle(),
        &mut record,
        &mut runtimes,
        None,
        &bound,
        None,
        None,
    )
    .unwrap_err();
    assert!(error.contains("cannot prove the requested community authority"));
    assert!(!error.contains("crash"));
    assert!(runtimes.is_empty());
    assert!(child.child_mut().try_wait().unwrap().is_none());
    assert!(
        crate::managed_agents::read_all_agent_runtime_receipts(app.handle())
            .iter()
            .any(|(_, candidate)| candidate == &receipt)
    );
    crate::managed_agents::remove_agent_runtime_receipt(app.handle(), &key);
}

#[test]
fn receipt_selection_refuses_ambiguous_unversioned_loopback_authority() {
    let mut receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://127.0.0.1:3000")
            .unwrap(),
    );
    receipt.authority_version = 0;
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));
    for requested_relay in [
        "ws://127.0.0.1:3000",
        "ws://localhost:3000",
        "ws://127.0.0.2:3000",
        "ws://[::1]:3000",
    ] {
        let requested =
            crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), requested_relay)
                .unwrap();

        let error = runtime::select_pair_runtime_receipt_with(
            vec![(path.clone(), receipt.clone())],
            &requested,
            "test-instance",
            |_| true,
            |_, _| true,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prove the requested community authority"),
            "legacy receipt must not prove {requested_relay}"
        );
    }
}

#[test]
fn receipt_selection_keeps_versioned_loopback_authorities_disjoint() {
    let receipt = receipt_fixture(
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://127.0.0.1:3000")
            .unwrap(),
    );
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));
    let requested =
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "ws://localhost:3000")
            .unwrap();

    let selected = runtime::select_pair_runtime_receipt_with(
        vec![(path, receipt)],
        &requested,
        "test-instance",
        |_| true,
        |_, _| true,
    )
    .unwrap();
    assert!(selected.is_none());
}

#[test]
fn receipt_selection_refuses_unversioned_non_loopback_lossy_urls() {
    let pubkey = "aa".repeat(32);
    for (stored_relay, requested_relay) in [
        ("wss://relay.example/room", "wss://relay.example/room"),
        ("wss://relay.example/room", "wss://relay.example/room/"),
        (
            "wss://relay.example/?mode=one",
            "wss://relay.example?mode=one",
        ),
        (
            "wss://relay.example/room?tail=",
            "wss://relay.example/room?tail=/",
        ),
    ] {
        let mut receipt = receipt_fixture(
            crate::managed_agents::ManagedAgentRuntimeKey::new(
                pubkey.clone(),
                "wss://relay.example",
            )
            .unwrap(),
        );
        receipt.authority_version = 0;
        receipt.key.relay_url = stored_relay.into();
        let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));
        let requested =
            crate::managed_agents::ManagedAgentRuntimeKey::new(pubkey.clone(), requested_relay)
                .unwrap();

        let error = runtime::select_pair_runtime_receipt_with(
            vec![(path, receipt)],
            &requested,
            "test-instance",
            |_| true,
            |_, _| true,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot prove the requested community authority"),
            "legacy {stored_relay} must not prove {requested_relay}"
        );
    }
}

#[test]
fn legacy_renderer_collapses_repeated_root_slashes_but_modern_keys_do_not() {
    let parsed = url::Url::parse("wss://relay.example//").unwrap();
    assert_eq!(parsed.path(), "//");
    assert_eq!(
        buzz_core_pkg::relay::normalize_relay_url("wss://relay.example//").unwrap(),
        "wss://relay.example"
    );
    let root =
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
            .unwrap();
    let repeated = crate::managed_agents::ManagedAgentRuntimeKey::new(
        "aa".repeat(32),
        "wss://relay.example//",
    )
    .unwrap();
    assert_ne!(root, repeated);
}

#[test]
fn receipt_selection_refuses_unversioned_root_authority() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
            .unwrap();
    let mut receipt = receipt_fixture(key.clone());
    receipt.authority_version = 0;
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));

    let error = runtime::select_pair_runtime_receipt_with(
        vec![(path, receipt)],
        &key,
        "test-instance",
        |_| true,
        |_, _| true,
    )
    .unwrap_err();
    assert!(error.contains("cannot prove the requested community authority"));
}

#[test]
fn repeated_root_request_is_refused_for_colliding_unversioned_receipt() {
    let pubkey = "aa".repeat(32);
    let stored =
        crate::managed_agents::ManagedAgentRuntimeKey::new(&pubkey, "wss://relay.example").unwrap();
    let requested =
        crate::managed_agents::ManagedAgentRuntimeKey::new(&pubkey, "wss://relay.example//")
            .unwrap();
    let mut receipt = receipt_fixture(stored);
    receipt.authority_version = 0;
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));

    let error = runtime::select_pair_runtime_receipt_with(
        vec![(path, receipt)],
        &requested,
        "test-instance",
        |_| true,
        |_, _| true,
    )
    .unwrap_err();
    assert!(error.contains("cannot prove the requested community authority"));
}

#[test]
fn receipt_selection_refuses_unknown_future_authority_version() {
    let key =
        crate::managed_agents::ManagedAgentRuntimeKey::new("aa".repeat(32), "wss://relay.example")
            .unwrap();
    let mut receipt = receipt_fixture(key.clone());
    receipt.authority_version = crate::managed_agents::RUNTIME_AUTHORITY_RECEIPT_VERSION + 1;
    let path = std::path::PathBuf::from(format!("{}.json", receipt.key.runtime_id()));

    let error = runtime::select_pair_runtime_receipt_with(
        vec![(path, receipt)],
        &key,
        "test-instance",
        |_| true,
        |_, _| true,
    )
    .unwrap_err();
    assert!(error.contains("cannot prove the requested community authority"));
}

// ── workspace pair-key resolution (summary/stop scoping) ────────────────
