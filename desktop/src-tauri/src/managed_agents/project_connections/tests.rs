use super::*;

fn scope() -> ProjectConnectionScope {
    ProjectConnectionScope {
        relay_url: "ws://127.0.0.1:3000".to_string(),
        operator_pubkey: "b".repeat(64),
        project_address: format!("30621:{}:portable-agents", "a".repeat(64)),
    }
}

fn stored_connection() -> StoredProjectConnection {
    StoredProjectConnection {
        id: "c".repeat(32),
        project_scope: scope(),
        name: "Analytics".to_string(),
        provider: "Local test".to_string(),
        capability_ids: vec!["mcp.tool.run_report".to_string()],
        command: "/usr/bin/true".to_string(),
        args: Vec::new(),
        env_keys: vec!["API_TOKEN".to_string()],
        discovered_tools: vec!["run_report".to_string()],
        health: ProjectConnectionHealth {
            status: ProjectConnectionHealthStatus::Ready,
            last_verified_at: Some(now_iso()),
            detail: None,
        },
        executable_sha256: "d".repeat(64),
        generation: "e".repeat(32),
        credential_generation: "f".repeat(32),
        created_at: now_iso(),
        updated_at: now_iso(),
    }
}

#[test]
fn project_scope_requires_canonical_relay_identity_and_coordinate() {
    assert_eq!(canonical_project_scope(&scope()).unwrap(), scope());
    let mut localhost = scope();
    localhost.relay_url = "ws://localhost:3000".to_string();
    assert_eq!(canonical_project_scope(&localhost).unwrap(), scope());
    let mut invalid = scope();
    invalid.project_address = "local-project-id".to_string();
    assert!(canonical_project_scope(&invalid).is_err());
    let mut invalid = scope();
    invalid.operator_pubkey = "not-a-key".to_string();
    assert!(canonical_project_scope(&invalid).is_err());

    let mut legacy = scope();
    legacy.project_address = format!("30617:{}:portable-agents", "a".repeat(64));
    assert_eq!(canonical_project_scope(&legacy).unwrap(), legacy);
}

#[test]
fn public_projection_omits_secret_values_and_internal_generation() {
    let public = ProjectConnection::from(stored_connection());
    let json = serde_json::to_value(public).unwrap();
    assert_eq!(json["envKeys"], serde_json::json!(["API_TOKEN"]));
    assert!(json.get("generation").is_none());
    assert!(!json.to_string().contains("private-generation"));
    assert!(!json.to_string().contains("secret-value"));
}

#[test]
fn connection_input_rejects_reserved_empty_and_oversized_secrets() {
    let valid = BTreeMap::from([("API_TOKEN".to_string(), "value".to_string())]);
    assert!(validate_connection_input("Analytics", "Local", "/bin/true", &[], &valid).is_ok());
    let reserved = BTreeMap::from([("BUZZ_PRIVATE_KEY".to_string(), "value".to_string())]);
    assert!(validate_connection_input("Analytics", "Local", "/bin/true", &[], &reserved).is_err());
    let empty = BTreeMap::from([("API_TOKEN".to_string(), String::new())]);
    assert!(validate_connection_input("Analytics", "Local", "/bin/true", &[], &empty).is_err());
}

#[test]
fn verified_secret_write_keeps_the_saved_generation() {
    use std::cell::Cell;

    let deleted = Cell::new(false);
    assert!(store_verified_secret(
        || Ok(()),
        || Ok(true),
        || {
            deleted.set(true);
            Ok(())
        },
    )
    .is_ok());
    assert!(!deleted.get());
}

#[test]
fn failed_secret_verification_removes_the_saved_generation() {
    use std::cell::Cell;

    let deleted = Cell::new(false);
    let error = store_verified_secret(
        || Ok(()),
        || Ok(false),
        || {
            deleted.set(true);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error, "Buzz could not verify the saved credentials.");
    assert!(deleted.get());
}

#[test]
fn secret_verification_error_removes_the_saved_generation() {
    use std::cell::Cell;

    let deleted = Cell::new(false);
    let error = store_verified_secret(
        || Ok(()),
        || Err("backend detail".to_string()),
        || {
            deleted.set(true);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error, "Buzz could not verify the saved credentials.");
    assert!(deleted.get());
}

#[test]
fn secret_write_error_removes_a_possible_partial_generation() {
    use std::cell::Cell;

    let verified = Cell::new(false);
    let deleted = Cell::new(false);
    let error = store_verified_secret(
        || Err("backend detail".to_string()),
        || {
            verified.set(true);
            Ok(true)
        },
        || {
            deleted.set(true);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        "Buzz could not save these credentials in the system keyring."
    );
    assert!(!verified.get());
    assert!(deleted.get());
}

#[test]
fn failed_secret_cleanup_is_reported_without_backend_details() {
    let error = store_verified_secret(
        || Ok(()),
        || Ok(false),
        || Err("backend detail".to_string()),
    )
    .unwrap_err();
    assert_eq!(
        error,
        "Buzz could not verify the saved credentials. \
         Buzz also could not remove the unverified credentials."
    );
    assert!(!error.contains("backend detail"));
}

#[test]
fn stale_ready_connection_is_presented_as_check_needed() {
    let mut connection = stored_connection();
    connection.health.last_verified_at = Some("2020-01-01T00:00:00Z".to_string());
    assert_eq!(
        health_for_display(connection).health.status,
        ProjectConnectionHealthStatus::CheckNeeded
    );
}

#[test]
fn stored_connection_rejects_invalid_ids_generations_and_fingerprints() {
    let mut connection = stored_connection();
    assert!(validate_stored_connection(&connection).is_ok());
    connection.id = "../outside".to_string();
    assert!(validate_stored_connection(&connection).is_err());

    let mut connection = stored_connection();
    connection.credential_generation = "not-a-generation".to_string();
    assert!(validate_stored_connection(&connection).is_err());

    let mut connection = stored_connection();
    connection.executable_sha256 = "not-a-fingerprint".to_string();
    assert!(validate_stored_connection(&connection).is_err());
}

#[test]
fn connection_lookup_cannot_cross_project_boundaries() {
    let connection = stored_connection();
    let store = ProjectConnectionStore {
        version: CONNECTION_STORE_VERSION,
        connections: vec![connection.clone()],
    };
    assert!(find_connection(&store, &connection.project_scope, &connection.id).is_ok());

    let mut other_project = connection.project_scope;
    other_project.project_address = format!("30621:{}:other-project", "a".repeat(64));
    assert!(find_connection(&store, &other_project, &connection.id).is_err());
}

#[cfg(unix)]
#[test]
fn connection_store_rejects_symlinks_and_non_owner_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt as _};

    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("connections.json");
    let target = dir.path().join("target.json");
    fs::write(&target, b"{}").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    symlink(&target, &store).unwrap();
    assert!(reject_unsafe_owner_file(&store).is_err());

    fs::remove_file(&store).unwrap();
    fs::write(&store, b"{}").unwrap();
    fs::set_permissions(&store, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(reject_unsafe_owner_file(&store).is_err());
    fs::set_permissions(&store, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(reject_unsafe_owner_file(&store).is_ok());
}
