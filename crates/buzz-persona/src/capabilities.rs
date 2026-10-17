//! Persona-owned harness inputs. These are assignments, not an access-control boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::persona::{Hooks, McpServerConfig};

/// Configuration carried by managed personas, portable snapshots and ACP launches.
/// Skill paths identify native skill directories; Buzz never rewrites their frontmatter.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonaCapabilities {
    /// Native skill directories, absolute or relative to the working directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<String>,
    /// Stdio MCP assignments. An identically named persona server replaces a global default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerConfig>,
    /// Buzz lifecycle scripts, separate from native harness hooks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<Hooks>,
    /// Explicit workspace. Relative references are resolved here, never against a random CWD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
}

impl std::fmt::Debug for PersonaCapabilities {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonaCapabilities")
            .field("skill_count", &self.skills.len())
            .field("mcp_count", &self.mcp_servers.len())
            .field("has_hooks", &self.hooks.is_some())
            .field("has_working_directory", &self.working_directory.is_some())
            .finish()
    }
}

impl PersonaCapabilities {
    /// Whether this record carries no overrides or assignments.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
            && self.mcp_servers.is_empty()
            && self.hooks.is_none()
            && self.working_directory.is_none()
    }

    /// Merge explicit global defaults with persona assignments. Native user settings
    /// remain a harness concern. Skills are a stable union; MCP names and hook events
    /// are overridden individually. An empty persona list inherits global defaults.
    pub fn merged(&self, persona: &Self) -> Self {
        let mut skills = self.skills.clone();
        for skill in &persona.skills {
            if !skills.contains(skill) {
                skills.push(skill.clone());
            }
        }
        let mut servers: BTreeMap<_, _> = self
            .mcp_servers
            .iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect();
        servers.extend(
            persona
                .mcp_servers
                .iter()
                .map(|s| (s.name.clone(), s.clone())),
        );
        let hooks = match (&self.hooks, &persona.hooks) {
            (Some(a), Some(b)) => Some(Hooks {
                on_start: b.on_start.clone().or_else(|| a.on_start.clone()),
                on_stop: b.on_stop.clone().or_else(|| a.on_stop.clone()),
                on_message: b.on_message.clone().or_else(|| a.on_message.clone()),
            }),
            (a, b) => b.clone().or_else(|| a.clone()),
        };
        Self {
            skills,
            mcp_servers: servers.into_values().collect(),
            hooks,
            working_directory: persona
                .working_directory
                .clone()
                .or_else(|| self.working_directory.clone()),
        }
    }

    /// Validate bounded configuration without opening files or reading process state.
    pub fn validate(&self) -> Result<(), String> {
        // This value is carried in one launch environment variable. Bound the
        // whole encoded value, including map keys, rather than each field alone.
        if serde_json::to_vec(self)
            .map_err(|_| "cannot serialize persona assignments")?
            .len()
            > 64 * 1024
        {
            return Err("persona assignments exceed the 64 KiB launch limit".into());
        }
        if self.skills.len() > 64 || self.mcp_servers.len() > 32 {
            return Err("at most 64 skill references and 32 MCP assignments are supported".into());
        }
        fn text(value: &str, field: &str, limit: usize) -> Result<(), String> {
            if value.trim().is_empty() || value.len() > limit || value.contains('\0') {
                return Err(format!("invalid {field}: empty, too long or contains NUL"));
            }
            Ok(())
        }
        for skill in &self.skills {
            text(skill, "skill reference", 4096)?;
        }
        if let Some(cwd) = &self.working_directory {
            text(cwd, "working directory", 4096)?;
            if !Path::new(cwd).is_absolute() {
                return Err("working directory must be absolute".into());
            }
        }
        let mut names = BTreeSet::new();
        for server in &self.mcp_servers {
            text(&server.name, "MCP name", 128)?;
            if !server
                .name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
            {
                return Err("MCP names must use letters, digits, underscores or hyphens".into());
            }
            if !names.insert(&server.name) {
                return Err(format!("duplicate MCP name: {}", server.name));
            }
            text(&server.command, "MCP command", 4096)?;
            if server.args.len() > 128 || server.env.len() > 128 {
                return Err("MCP assignment is too large".into());
            }
            for arg in &server.args {
                if arg.len() > 8192 || arg.contains('\0') {
                    return Err("invalid MCP argument".into());
                }
            }
            for (key, value) in &server.env {
                if key.is_empty()
                    || !key.bytes().enumerate().all(|(i, c)| {
                        c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                    })
                {
                    return Err("invalid MCP environment name".into());
                }
                if value.len() > 8192 || value.contains('\0') {
                    return Err("invalid MCP environment value".into());
                }
            }
        }
        if let Some(hooks) = &self.hooks {
            for hook in [&hooks.on_start, &hooks.on_stop, &hooks.on_message]
                .into_iter()
                .flatten()
            {
                text(hook, "hook path", 4096)?;
            }
        }
        Ok(())
    }

    /// Resolve an explicit workspace against a caller-provided default. This never
    /// reads HOME and refuses missing directories rather than silently falling back.
    /// Without an explicit workspace the default is returned unchanged, so agents
    /// with no assignment keep their existing launch directory spelling (Windows
    /// canonicalization would otherwise add a `\\?\` prefix to every launch).
    pub fn resolve_cwd(&self, default: &Path) -> Result<PathBuf, String> {
        let cwd = self
            .working_directory
            .as_deref()
            .map(Path::new)
            .unwrap_or(default);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err("persona working directory must be an existing absolute directory".into());
        }
        if self.working_directory.is_none() {
            return Ok(default.to_path_buf());
        }
        cwd.canonicalize()
            .map_err(|e| format!("cannot resolve persona working directory: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_preserves_baseline_and_overrides_named_server_and_hook() {
        let base: PersonaCapabilities = serde_json::from_value(serde_json::json!({
            "skills":["baseline"],"mcp_servers":[{"name":"report","command":"old"}],
            "hooks":{"on_start":"start","on_stop":"stop"}
        }))
        .unwrap();
        let persona: PersonaCapabilities = serde_json::from_value(serde_json::json!({
            "skills":["review","baseline"],"mcp_servers":[{"name":"report","command":"new","args":["--safe"],"env":{"ROLE":"engineer"}}],
            "hooks":{"on_start":"persona-start"}
        })).unwrap();
        let merged = base.merged(&persona);
        assert_eq!(merged.skills, ["baseline", "review"]);
        assert_eq!(merged.mcp_servers[0].command, "new");
        assert_eq!(merged.mcp_servers[0].args, ["--safe"]);
        assert_eq!(merged.hooks.unwrap().on_stop.as_deref(), Some("stop"));
    }

    #[test]
    fn roundtrip_keeps_native_references_and_structured_environment() {
        let value = serde_json::json!({"skills":["skills/日本語"],"mcp_servers":[{"name":"probe","command":"node","args":["probe.mjs"],"env":{"VALUE":"two words"}}]});
        let config: PersonaCapabilities = serde_json::from_value(value).unwrap();
        config.validate().unwrap();
        assert_eq!(
            serde_json::from_str::<PersonaCapabilities>(&serde_json::to_string(&config).unwrap())
                .unwrap(),
            config
        );
    }

    #[cfg(unix)]
    #[test]
    fn default_workspace_spelling_is_preserved_and_explicit_workspace_is_resolved() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // No assignment: the caller's default launch directory is used verbatim.
        assert_eq!(
            PersonaCapabilities::default().resolve_cwd(&link).unwrap(),
            link
        );
        let explicit = PersonaCapabilities {
            working_directory: Some(link.to_string_lossy().into_owned()),
            ..Default::default()
        };
        assert_eq!(
            explicit.resolve_cwd(Path::new("/")).unwrap(),
            real.canonicalize().unwrap()
        );
        assert!(PersonaCapabilities::default()
            .resolve_cwd(&root.path().join("absent"))
            .is_err());
    }

    #[test]
    fn invalid_configuration_is_rejected_without_echoing_environment_values() {
        for value in [
            serde_json::json!({"working_directory":"relative"}),
            serde_json::json!({"skills":vec!["a".repeat(4096);17]}),
            serde_json::json!({"mcp_servers":[{"name":"x","command":"node"},{"name":"x","command":"node"}]}),
            serde_json::json!({"mcp_servers":[{"name":"x","command":"node","env":{"INVALID-NAME":"secret-value"}}]}),
        ] {
            let config: PersonaCapabilities = serde_json::from_value(value).unwrap();
            let err = config.validate().unwrap_err();
            assert!(!err.contains("secret-value"));
        }
    }
}
