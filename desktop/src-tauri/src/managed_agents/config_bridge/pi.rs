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
}
