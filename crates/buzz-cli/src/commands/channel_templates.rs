//! Desktop-local channel template loading for `buzz channels create --template`.
//!
//! Templates live in a JSON file the desktop app owns
//! (`<app-data>/templates/channel-templates.json`); this module duplicates the
//! wire shape (`desktop/src-tauri/src/templates/types.rs`) rather than sharing
//! a crate, since buzz-cli and desktop-tauri are independent crates and the
//! shape is small and stable. Only the fields the CLI needs to read are kept.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::CliError;

/// Tauri bundle identifier for the production desktop app. `dirs::data_dir()`
/// joined with this segment matches `app.path().app_data_dir()` exactly
/// (Tauri resolves app-data as the platform data dir plus the identifier).
const PROD_BUNDLE_IDENTIFIER: &str = "xyz.block.buzz.app";

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelTemplateRecord {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_channel_type")]
    pub channel_type: String,
    #[serde(default = "default_visibility")]
    pub visibility: String,
    #[serde(default)]
    pub canvas_template: Option<String>,
    #[serde(default)]
    pub agents: TemplateAgentRoster,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateAgentRoster {
    #[serde(default)]
    pub personas: Vec<TemplateAgentEntry>,
    #[serde(default)]
    pub teams: Vec<TemplateTeamEntry>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateAgentEntry {
    pub persona_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateTeamEntry {
    pub team_id: String,
}

fn default_channel_type() -> String {
    "stream".to_string()
}

fn default_visibility() -> String {
    "open".to_string()
}

/// Resolve the desktop app's `channel-templates.json` path.
///
/// `override_path` (from `--templates-file`) always wins — useful for the dev
/// store or tests. Otherwise defaults to the prod bundle's app-data dir:
/// `<platform-data-dir>/xyz.block.buzz.app/templates/channel-templates.json`.
pub fn resolve_templates_path(override_path: Option<&str>) -> Result<PathBuf, CliError> {
    if let Some(p) = override_path {
        return Ok(PathBuf::from(p));
    }
    let data_dir = dirs::data_dir().ok_or_else(|| {
        CliError::Other("could not resolve platform app-data directory".to_string())
    })?;
    Ok(data_dir
        .join(PROD_BUNDLE_IDENTIFIER)
        .join("templates")
        .join("channel-templates.json"))
}

/// Load and parse the channel-templates store from `path`.
fn load_templates(path: &Path) -> Result<Vec<ChannelTemplateRecord>, CliError> {
    if !path.exists() {
        return Err(CliError::NotFound(format!(
            "no channel templates store found at {} (create a template in Buzz Desktop first, \
             or pass --templates-file)",
            path.display()
        )));
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| CliError::Other(format!("failed to read {}: {e}", path.display())))?;
    serde_json::from_str(&content)
        .map_err(|e| CliError::Other(format!("failed to parse {}: {e}", path.display())))
}

/// Load the templates store and find the template matching `name`
/// (case-insensitive, exact match). Errors list available names if not found.
pub fn find_template(path: &Path, name: &str) -> Result<ChannelTemplateRecord, CliError> {
    let templates = load_templates(path)?;
    let needle = name.to_ascii_lowercase();
    if let Some(t) = templates
        .into_iter()
        .find(|t| t.name.to_ascii_lowercase() == needle)
    {
        return Ok(t);
    }
    Err(CliError::NotFound(format!(
        "no channel template named '{name}' (available: {})",
        available_names(path)?
    )))
}

fn available_names(path: &Path) -> Result<String, CliError> {
    let templates = load_templates(path)?;
    if templates.is_empty() {
        return Ok("<none>".to_string());
    }
    Ok(templates
        .iter()
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_store(json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("tempfile");
        f.write_all(json.as_bytes()).expect("write");
        f
    }

    #[test]
    fn resolve_templates_path_honors_override() {
        let path = resolve_templates_path(Some("/tmp/custom.json")).unwrap();
        assert_eq!(path, PathBuf::from("/tmp/custom.json"));
    }

    #[test]
    fn resolve_templates_path_defaults_to_prod_bundle() {
        let path = resolve_templates_path(None).unwrap();
        assert!(path.ends_with("xyz.block.buzz.app/templates/channel-templates.json"));
    }

    #[test]
    fn find_template_matches_case_insensitive() {
        let f = write_store(r#"[{"id":"t1","name":"Buzz Team","createdAt":"x","updatedAt":"x"}]"#);
        let t = find_template(f.path(), "buzz team").expect("found");
        assert_eq!(t.name, "Buzz Team");
        assert_eq!(t.channel_type, "stream");
        assert_eq!(t.visibility, "open");
    }

    #[test]
    fn find_template_not_found_lists_available_names() {
        let f = write_store(
            r#"[{"id":"t1","name":"Buzz Team","createdAt":"x","updatedAt":"x"},
                {"id":"t2","name":"Standup","createdAt":"x","updatedAt":"x"}]"#,
        );
        let err = find_template(f.path(), "nope").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Buzz Team"));
        assert!(msg.contains("Standup"));
    }

    #[test]
    fn find_template_missing_store_is_not_found() {
        let err = find_template(Path::new("/nonexistent/channel-templates.json"), "x").unwrap_err();
        assert!(matches!(err, CliError::NotFound(_)));
    }

    #[test]
    fn load_templates_parses_full_roster() {
        let f = write_store(
            r##"[{
                "id":"t1","name":"Buzz Team","channel_type":"forum","visibility":"private",
                "canvas_template":"# {channel.name}",
                "agents":{"personas":[{"personaId":"builtin:fizz"}],"teams":[{"teamId":"team-1"}]},
                "created_at":"x","updated_at":"x"
            }]"##,
        );
        let t = find_template(f.path(), "Buzz Team").expect("found");
        assert_eq!(t.channel_type, "forum");
        assert_eq!(t.visibility, "private");
        assert_eq!(t.canvas_template.as_deref(), Some("# {channel.name}"));
        assert_eq!(t.agents.personas.len(), 1);
        assert_eq!(t.agents.personas[0].persona_id, "builtin:fizz");
        assert_eq!(t.agents.teams.len(), 1);
        assert_eq!(t.agents.teams[0].team_id, "team-1");
    }
}
