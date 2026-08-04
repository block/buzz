//! Bounded persistence and full validation for Project connection metadata.

use super::*;

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub(super) fn validate_stored_connection(
    connection: &StoredProjectConnection,
) -> Result<(), String> {
    if !is_lower_hex(&connection.id, 32)
        || !is_lower_hex(&connection.generation, 32)
        || !is_lower_hex(&connection.credential_generation, 32)
        || !is_lower_hex(&connection.executable_sha256, 64)
        || canonical_project_scope(&connection.project_scope)? != connection.project_scope
        || !Path::new(&connection.command).is_absolute()
    {
        return Err("Project connection metadata is invalid.".to_string());
    }
    let placeholder_env = connection
        .env_keys
        .iter()
        .map(|key| (key.clone(), "stored".to_string()))
        .collect();
    validate_connection_input(
        &connection.name,
        &connection.provider,
        &connection.command,
        &connection.args,
        &placeholder_env,
    )
    .map_err(|_| "Project connection metadata is invalid.".to_string())?;
    if connection
        .env_keys
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
        || connection
            .discovered_tools
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || connection.discovered_tools.len() > buzz_agent_pkg::MAX_MCP_TOOLS_PER_SESSION
    {
        return Err("Project connection metadata is invalid.".to_string());
    }
    let server_name = connection_mcp_server_name(&connection.id);
    if connection
        .discovered_tools
        .iter()
        .any(|tool| !buzz_agent_pkg::supports_mcp_server_tool_name(&server_name, tool))
    {
        return Err("Project connection metadata is invalid.".to_string());
    }
    let expected_capabilities = connection
        .discovered_tools
        .iter()
        .map(|tool| format!("mcp.tool.{tool}"))
        .collect::<Vec<_>>();
    if connection.capability_ids != expected_capabilities {
        return Err("Project connection metadata is invalid.".to_string());
    }
    if chrono::DateTime::parse_from_rfc3339(&connection.created_at).is_err()
        || chrono::DateTime::parse_from_rfc3339(&connection.updated_at).is_err()
        || connection
            .health
            .last_verified_at
            .as_deref()
            .is_some_and(|value| chrono::DateTime::parse_from_rfc3339(value).is_err())
        || connection.health.detail.as_deref().is_some_and(|detail| {
            detail.len() > MAX_HEALTH_DETAIL_BYTES || detail.chars().any(char::is_control)
        })
        || (connection.health.status == ProjectConnectionHealthStatus::Ready
            && (connection.health.last_verified_at.is_none()
                || connection.discovered_tools.is_empty()))
    {
        return Err("Project connection metadata is invalid.".to_string());
    }
    Ok(())
}

fn connection_store_path(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
) -> Result<PathBuf, String> {
    Ok(workspace_connection_dir(app, scope)?.join("connections.json"))
}

pub(super) fn read_bounded_file(path: &Path, max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take((max_bytes + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file exceeds size limit",
        ));
    }
    Ok(bytes)
}

fn validate_store(
    store: &ProjectConnectionStore,
    scope: &ProjectConnectionScope,
) -> Result<(), String> {
    if store.version != CONNECTION_STORE_VERSION {
        return Err(format!(
            "unsupported Project connection store version {}",
            store.version
        ));
    }
    if store.connections.len() > MAX_CONNECTIONS {
        return Err("Project connection store exceeds its connection limit".to_string());
    }
    let canonical_workspace = canonical_project_scope(scope)?;
    let mut ids = BTreeSet::new();
    for connection in &store.connections {
        validate_stored_connection(connection)?;
        if connection.project_scope.relay_url != canonical_workspace.relay_url
            || connection.project_scope.operator_pubkey != canonical_workspace.operator_pubkey
            || !ids.insert(connection.id.as_str())
        {
            return Err("Project connection metadata is invalid.".to_string());
        }
    }
    Ok(())
}

pub(super) fn load_store_unlocked(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
) -> Result<ProjectConnectionStore, String> {
    let path = connection_store_path(app, scope)?;
    reject_unsafe_owner_file(&path)?;
    let bytes = match read_bounded_file(&path, MAX_CONNECTION_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProjectConnectionStore::default());
        }
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Err("Project connection store exceeds its size limit".to_string());
        }
        Err(error) => {
            return Err(format!(
                "failed to read Project connections from {}: {error}",
                path.display()
            ));
        }
    };
    let store: ProjectConnectionStore = serde_json::from_slice(&bytes)
        .map_err(|error| format!("failed to parse Project connections: {error}"))?;
    validate_store(&store, scope)?;
    Ok(store)
}

pub(super) fn save_store_unlocked(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
    store: &ProjectConnectionStore,
) -> Result<(), String> {
    validate_store(store, scope)?;
    let path = connection_store_path(app, scope)?;
    reject_unsafe_owner_file(&path)?;
    let bytes = serde_json::to_vec_pretty(store)
        .map_err(|error| format!("failed to serialize Project connections: {error}"))?;
    if bytes.len() > MAX_CONNECTION_STORE_BYTES {
        return Err("Project connection store exceeds its size limit".to_string());
    }
    atomic_write_json_restricted(&path, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_rejects_duplicate_ids_and_cross_workspace_records() {
        let connection = super::super::tests::stored_connection();
        let duplicates = ProjectConnectionStore {
            version: CONNECTION_STORE_VERSION,
            connections: vec![connection.clone(), connection.clone()],
        };
        assert!(validate_store(&duplicates, &connection.project_scope).is_err());

        let mut foreign = connection.clone();
        foreign.project_scope.operator_pubkey = "f".repeat(64);
        let store = ProjectConnectionStore {
            version: CONNECTION_STORE_VERSION,
            connections: vec![foreign],
        };
        assert!(validate_store(&store, &connection.project_scope).is_err());
    }
}
