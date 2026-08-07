//! Inject Fieldcraft env into managed-agent children.
//!
//! Fieldcraft is a CLI memory system agents call via shell (`fieldcraft query` /
//! `write`). The shared server for this fleet lives on the Mac Mini
//! (`manuelas-mac-mini.taila1f329.ts.net:7777`), configured in
//! `~/.fieldcraft/env`. Agents spawned by Desktop do not run an interactive
//! shell, so they would not otherwise see that file unless fieldcraft itself
//! loads it.
//!
//! At spawn we load `~/.fieldcraft/env` (`export KEY=value` lines) and apply
//! every `FIELDCRAFT_*` var onto the child. We do **not** rewrite the server
//! URL to loopback: local listeners on this laptop are not the authority;
//! the Mac Mini is.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Load Fieldcraft env for a managed-agent child from `~/.fieldcraft/env`.
pub(crate) fn fieldcraft_env_for_agent() -> BTreeMap<String, String> {
    load_fieldcraft_env_file().unwrap_or_default()
}

/// Apply Fieldcraft env onto a child command (no-op when nothing is configured).
pub(crate) fn apply_fieldcraft_env(command: &mut std::process::Command) {
    for (key, value) in fieldcraft_env_for_agent() {
        command.env(key, value);
    }
}

fn fieldcraft_env_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".fieldcraft").join("env"))
}

fn load_fieldcraft_env_file() -> Option<BTreeMap<String, String>> {
    let path = fieldcraft_env_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_fieldcraft_env(&text))
}

/// Parse `export KEY=value` / `KEY=value` lines. Comments and blanks skipped.
fn parse_fieldcraft_env(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if !key.starts_with("FIELDCRAFT_") {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
        out.insert(key.to_string(), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_export_lines() {
        let text = r#"
# comment
export FIELDCRAFT_SERVER_URL=http://example.ts.net:7777
export FIELDCRAFT_AUTH_TOKEN=secret
FIELDCRAFT_DISTILLER=none
IGNORE_ME=1
"#;
        let map = parse_fieldcraft_env(text);
        assert_eq!(
            map.get("FIELDCRAFT_SERVER_URL").map(String::as_str),
            Some("http://example.ts.net:7777")
        );
        assert_eq!(
            map.get("FIELDCRAFT_AUTH_TOKEN").map(String::as_str),
            Some("secret")
        );
        assert_eq!(
            map.get("FIELDCRAFT_DISTILLER").map(String::as_str),
            Some("none")
        );
        assert!(!map.contains_key("IGNORE_ME"));
    }
}
