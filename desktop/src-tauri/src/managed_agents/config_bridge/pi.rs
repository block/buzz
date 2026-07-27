use super::types::RuntimeFileConfig;
use std::path::PathBuf;

/// Read pi's harness settings from `~/.pi/agent/settings.json`
/// (or `$PI_CODING_AGENT_DIR/settings.json`).
///
/// Pi's settings.json holds harness behavior (steering, transport, trust) —
/// not model/provider, which live in pi's credential store and models.json.
/// Everything is surfaced read-only via `extra`; normalized fields stay None.
pub(super) fn read_config_file() -> Option<RuntimeFileConfig> {
    let path = pi_settings_path()?;
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

/// Pi's config directory: `$PI_CODING_AGENT_DIR` if set, else `~/.pi/agent`.
pub(crate) fn pi_agent_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::home_dir().map(|home| home.join(".pi").join("agent"))
}

fn pi_settings_path() -> Option<PathBuf> {
    pi_agent_dir().map(|dir| dir.join("settings.json"))
}

/// Ensure `<workdir>/.pi/mcp.json` registers `buzz-dev-mcp` for pi's MCP
/// extension. Merge-preserving (foreign servers and keys survive), idempotent
/// (no rewrite when already correct), and refuses to clobber malformed JSON.
///
/// The file contains no secrets: `buzz-dev-mcp` reads its relay URL and key
/// from the process environment injected by `buzz-acp`, and is resolved via
/// the augmented child PATH.
pub(super) fn ensure_workdir_mcp_json(workdir: &std::path::Path) -> Result<(), String> {
    let pi_dir = workdir.join(".pi");
    std::fs::create_dir_all(&pi_dir)
        .map_err(|e| format!("create {}: {e}", pi_dir.display()))?;
    let path = pi_dir.join("mcp.json");

    let mut root: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
            format!(
                "existing {} is not valid JSON ({e}); refusing to overwrite",
                path.display()
            )
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(err) => return Err(format!("read {}: {err}", path.display())),
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

    let serialized = serde_json::to_string_pretty(&root)
        .map_err(|e| format!("serialize {}: {e}", path.display()))?;
    std::fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))
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
    fn pi_agent_dir_honors_env_override() {
        // PI_CODING_AGENT_DIR overrides ~/.pi/agent (pi's own convention).
        // Serialize env mutation isn't needed: this test sets and removes
        // within one test; the suite has no other reader of this var.
        std::env::set_var("PI_CODING_AGENT_DIR", "/tmp/pi-test-agent-dir");
        let dir = pi_agent_dir();
        std::env::remove_var("PI_CODING_AGENT_DIR");
        assert_eq!(dir, Some(PathBuf::from("/tmp/pi-test-agent-dir")));
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
        assert_eq!(first_mtime, second_mtime, "no rewrite when content is correct");
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
}
