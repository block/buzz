use super::types::RuntimeFileConfig;
use std::path::PathBuf;

/// Read pi's harness settings from `~/.pi/agent/settings.json`
/// (or `$PI_CODING_AGENT_DIR/settings.json`).
///
/// Pi's settings.json holds harness behavior (steering, transport, trust) —
/// not model/provider, which live in pi's credential store and models.json.
/// Everything is surfaced read-only via `extra`; normalized fields stay None.
pub(super) fn read_config_file(record_agent_dir: Option<&str>) -> Option<RuntimeFileConfig> {
    let path = pi_settings_path(record_agent_dir)?;
    let raw = std::fs::read_to_string(path).ok()?;
    parse_pi_settings(&raw)
}

fn parse_pi_settings(json_str: &str) -> Option<RuntimeFileConfig> {
    let value: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let extra = super::schema_walker::extract_config_fields(&value, &[]);
    Some(RuntimeFileConfig {
        extra,
        ..Default::default()
    })
}

/// Pi's config directory: the agent record's `PI_CODING_AGENT_DIR`, then the
/// desktop process environment, then `~/.pi/agent`.
pub(crate) fn pi_agent_dir(record_agent_dir: Option<&str>) -> Option<PathBuf> {
    if let Some(dir) = record_agent_dir.filter(|dir| !dir.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|home| home.join(".pi").join("agent"))
}

/// Resolve pi's `settings.json` path using the agent-specific directory override.
pub(crate) fn pi_settings_path(record_agent_dir: Option<&str>) -> Option<PathBuf> {
    pi_agent_dir(record_agent_dir).map(|dir| dir.join("settings.json"))
}

/// Ensure `<workdir>/.pi/mcp.json` registers `buzz-dev-mcp` for pi's MCP
/// extension. Merge-preserving (foreign servers and keys survive), idempotent
/// (no rewrite when already correct), and refuses to clobber malformed JSON.
///
/// The file contains no secrets: `buzz-dev-mcp` reads its relay URL and key
/// from the process environment injected by `buzz-acp`, and is resolved via
/// the augmented child PATH.
pub(super) fn ensure_workdir_mcp_json(workdir: &std::path::Path) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;
    use std::io::Write;

    let pi_dir = workdir.join(".pi");
    match std::fs::symlink_metadata(&pi_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "refusing to use symlinked pi config directory {}",
                pi_dir.display()
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(format!("{} is not a directory", pi_dir.display()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&pi_dir)
                .map_err(|error| format!("create {}: {error}", pi_dir.display()))?;
        }
        Err(error) => return Err(format!("inspect {}: {error}", pi_dir.display())),
    }

    // Re-check after creation so a racing replacement cannot turn the directory
    // into a symlink before the config file is inspected.
    let pi_dir_metadata = std::fs::symlink_metadata(&pi_dir)
        .map_err(|error| format!("inspect {}: {error}", pi_dir.display()))?;
    if pi_dir_metadata.file_type().is_symlink() || !pi_dir_metadata.is_dir() {
        return Err(format!(
            "refusing to use non-directory or symlinked pi config path {}",
            pi_dir.display()
        ));
    }

    let path = pi_dir.join("mcp.json");
    let path_exists = match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "refusing to overwrite symlinked pi MCP config {}",
                path.display()
            ));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!("{} is not a regular file", path.display()));
        }
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("inspect {}: {error}", path.display())),
    };

    let mut root: serde_json::Value = if path_exists {
        let raw = std::fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        serde_json::from_str(&raw).map_err(|error| {
            format!(
                "existing {} is not valid JSON ({error}); refusing to overwrite",
                path.display()
            )
        })?
    } else {
        serde_json::json!({})
    };

    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| format!("{} root is not a JSON object", path.display()))?;
    let servers = root_obj
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    let servers_obj = servers
        .as_object_mut()
        .ok_or_else(|| format!("{} mcpServers is not a JSON object", path.display()))?;

    let desired = serde_json::json!({ "command": "buzz-dev-mcp" });
    if servers_obj.get("buzz") == Some(&desired) {
        return Ok(());
    }
    servers_obj.insert("buzz".to_string(), desired);

    let serialized = serde_json::to_vec_pretty(&root)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    let mut temp = AtomicWriteFile::open(&path)
        .map_err(|error| format!("open {} for atomic write: {error}", path.display()))?;
    temp.write_all(&serialized)
        .map_err(|error| format!("write {}: {error}", path.display()))?;
    temp.commit()
        .map_err(|error| format!("commit {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_settings_surfaces_fields_as_extra() {
        let json = r#"{
            "steeringMode": "one-at-a-time",
            "transport": "auto",
            "defaultProjectTrust": "ask"
        }"#;
        let cfg = parse_pi_settings(json).unwrap();
        assert_eq!(
            cfg.extra.get("steeringMode").map(String::as_str),
            Some("one-at-a-time")
        );
        assert_eq!(cfg.extra.get("transport").map(String::as_str), Some("auto"));
        // Pi settings.json carries no model/provider — normalized fields stay None.
        assert!(cfg.model.is_none());
        assert!(cfg.provider.is_none());
        assert!(cfg.system_prompt.is_none());
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(parse_pi_settings("{{{{not json").is_none());
    }

    #[test]
    fn pi_agent_dir_honors_record_env_override() {
        assert_eq!(
            pi_agent_dir(Some("/tmp/pi-test-agent-dir")),
            Some(PathBuf::from("/tmp/pi-test-agent-dir"))
        );
    }

    #[test]
    fn ensure_mcp_json_creates_file_with_buzz_server() {
        let dir = tempfile::tempdir().unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let raw = std::fs::read_to_string(dir.path().join(".pi/mcp.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            json["mcpServers"]["buzz"]["command"],
            serde_json::json!("buzz-dev-mcp")
        );
    }

    #[test]
    fn ensure_mcp_json_preserves_foreign_servers() {
        let dir = tempfile::tempdir().unwrap();
        let pi_dir = dir.path().join(".pi");
        std::fs::create_dir_all(&pi_dir).unwrap();
        std::fs::write(
            pi_dir.join("mcp.json"),
            r#"{"mcpServers": {"github": {"command": "gh-mcp"}}, "otherKey": 1}"#,
        )
        .unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let raw = std::fs::read_to_string(pi_dir.join("mcp.json")).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            json["mcpServers"]["github"]["command"],
            serde_json::json!("gh-mcp"),
            "foreign server must survive the merge"
        );
        assert_eq!(json["otherKey"], serde_json::json!(1));
        assert_eq!(
            json["mcpServers"]["buzz"]["command"],
            serde_json::json!("buzz-dev-mcp")
        );
    }

    #[test]
    fn ensure_mcp_json_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let path = dir.path().join(".pi/mcp.json");
        let first_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        ensure_workdir_mcp_json(dir.path()).unwrap();
        let second_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            first_mtime, second_mtime,
            "no rewrite when content is correct"
        );
    }

    #[test]
    fn ensure_mcp_json_refuses_to_clobber_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let pi_dir = dir.path().join(".pi");
        std::fs::create_dir_all(&pi_dir).unwrap();
        std::fs::write(pi_dir.join("mcp.json"), "{{{{not json").unwrap();
        assert!(ensure_workdir_mcp_json(dir.path()).is_err());
        // Original content untouched.
        assert_eq!(
            std::fs::read_to_string(pi_dir.join("mcp.json")).unwrap(),
            "{{{{not json"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_mcp_json_rejects_symlinked_pi_directory() {
        use std::os::unix::fs::symlink;

        let workdir = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        symlink(target.path(), workdir.path().join(".pi")).unwrap();

        let error = ensure_workdir_mcp_json(workdir.path()).unwrap_err();
        assert!(error.contains("symlink"), "unexpected error: {error}");
        assert!(!target.path().join("mcp.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_mcp_json_rejects_symlinked_config_file() {
        use std::os::unix::fs::symlink;

        let workdir = tempfile::tempdir().unwrap();
        let pi_dir = workdir.path().join(".pi");
        std::fs::create_dir(&pi_dir).unwrap();
        let target = workdir.path().join("outside.json");
        std::fs::write(&target, r#"{"mcpServers": {}}"#).unwrap();
        symlink(&target, pi_dir.join("mcp.json")).unwrap();

        let error = ensure_workdir_mcp_json(workdir.path()).unwrap_err();
        assert!(error.contains("symlink"), "unexpected error: {error}");
        assert_eq!(
            std::fs::read_to_string(target).unwrap(),
            r#"{"mcpServers": {}}"#
        );
    }
}
