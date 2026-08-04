use super::*;

fn scope() -> ProjectConnectionScope {
    ProjectConnectionScope {
        relay_url: "ws://127.0.0.1:3000".to_string(),
        operator_pubkey: "b".repeat(64),
        project_address: format!("30621:{}:portable-agents", "a".repeat(64)),
    }
}

pub(super) fn stored_connection() -> StoredProjectConnection {
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
    let oversized = BTreeMap::from([("API_TOKEN".to_string(), "x".repeat(MAX_SECRET_BYTES + 1))]);
    assert!(validate_connection_input("Analytics", "Local", "/bin/true", &[], &oversized).is_err());
}

#[test]
fn connection_input_rejects_case_collisions_without_echoing_pasted_secrets() {
    let collision = BTreeMap::from([
        ("API_TOKEN".to_string(), "one".to_string()),
        ("api_token".to_string(), "two".to_string()),
    ]);
    assert!(validate_connection_input("Analytics", "Local", "/bin/true", &[], &collision).is_err());

    let pasted = BTreeMap::from([(
        "ANTHROPIC_API_KEY=sk-must-not-echo".to_string(),
        "ignored".to_string(),
    )]);
    let error =
        validate_connection_input("Analytics", "Local", "/bin/true", &[], &pasted).unwrap_err();
    assert!(!error.contains("sk-must-not-echo"));
    assert!(error.contains("ANTHROPIC_API_KEY"));
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
fn stored_connection_revalidates_all_runtime_facing_fields() {
    let mut connection = stored_connection();
    connection.name = "spoofed\nname".to_string();
    assert!(validate_stored_connection(&connection).is_err());

    let mut connection = stored_connection();
    connection.args = vec!["x".repeat(MAX_ARG_BYTES + 1)];
    assert!(validate_stored_connection(&connection).is_err());

    let mut connection = stored_connection();
    connection.env_keys = vec!["API_TOKEN".to_string(), "api_token".to_string()];
    assert!(validate_stored_connection(&connection).is_err());

    let mut connection = stored_connection();
    connection.discovered_tools = vec!["unsupported.dotted".to_string()];
    connection.capability_ids = vec!["mcp.tool.unsupported.dotted".to_string()];
    assert!(validate_stored_connection(&connection).is_err());

    let mut connection = stored_connection();
    connection.capability_ids = vec!["mcp.tool.different".to_string()];
    assert!(validate_stored_connection(&connection).is_err());
}

#[test]
fn connection_store_size_is_bounded_before_deserialization() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("connections.json");
    fs::write(&path, vec![b' '; MAX_CONNECTION_STORE_BYTES + 1]).unwrap();
    let error = read_bounded_file(&path, MAX_CONNECTION_STORE_BYTES).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
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

#[test]
fn generated_server_name_preserves_room_for_mcp_tool_names() {
    let server_name = connection_mcp_server_name(&"c".repeat(32));
    assert_eq!(server_name, "project_cccccccccccc");
    assert!(buzz_agent_pkg::supports_mcp_server_tool_name(
        &server_name,
        "analytics_weekly_summary"
    ));
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
