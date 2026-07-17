//! One-way migration from the retired bundled `buzz-agent` desktop runtime to
//! the bundled Goose ACP runtime.

use std::path::Path;

use tauri::Manager as _;

use super::{canonical_dev_data_dir, patch_json_records};

const OLD_RUNTIME: &str = "buzz-agent";
const NEW_RUNTIME: &str = "goose";
const OLD_COMMAND: &str = "buzz-agent";
const NEW_COMMAND: &str = "goose-acp";
const OLD_MCP_COMMAND: &str = "buzz-dev-mcp";

/// Migrate exact built-in Buzz Agent references to the bundled Goose runtime.
///
/// Custom commands and paths are preserved. The migration is idempotent and
/// covers both unified JSON records and directory-backed team personas.
pub fn migrate_buzz_agent_to_goose(app: &tauri::AppHandle) {
    let Ok(current_dir) = app.path().app_data_dir() else {
        return;
    };
    let mut dirs = vec![current_dir.clone()];
    if let Some(canonical) = canonical_dev_data_dir(&current_dir) {
        if canonical.exists() && canonical != current_dir {
            dirs.push(canonical);
        }
    }

    for dir in dirs {
        let agents_dir = dir.join("agents");
        for path in [
            agents_dir.join("managed-agents.json"),
            agents_dir.join("personas.json"),
        ] {
            if path.exists() {
                migrate_json_file(&path);
            }
        }

        let teams_dir = agents_dir.join("teams");
        if teams_dir.exists() && !teams_dir.is_symlink() {
            migrate_team_personas(&teams_dir);
        }
    }
}

fn replace_exact(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    field: &str,
    old: &str,
    new: &str,
) -> bool {
    if obj.get(field).and_then(|value| value.as_str()) != Some(old) {
        return false;
    }
    obj.insert(
        field.to_string(),
        serde_json::Value::String(new.to_string()),
    );
    true
}

fn migrate_json_file(path: &Path) {
    patch_json_records(path, |obj| {
        let mut changed = replace_exact(obj, "runtime", OLD_RUNTIME, NEW_RUNTIME);
        changed |= replace_exact(obj, "agent_command", OLD_COMMAND, NEW_COMMAND);
        changed |= replace_exact(obj, "agent_command_override", OLD_COMMAND, NEW_COMMAND);

        if changed {
            // The retired runtime alone used this in-process MCP sidecar. Goose
            // supplies developer tools itself and must not inherit the old one.
            replace_exact(obj, "mcp_command", OLD_MCP_COMMAND, "");
        }
        changed
    });
}

fn rewrite_persona_runtime(content: &str) -> Option<String> {
    let (frontmatter, body) = buzz_persona_pkg::persona::split_frontmatter(content).ok()?;
    let mut value = serde_yaml::from_str::<serde_yaml::Value>(frontmatter).ok()?;
    let mapping = value.as_mapping_mut()?;
    let runtime = mapping.get_mut(serde_yaml::Value::String("runtime".to_string()))?;
    if runtime.as_str()? != OLD_RUNTIME {
        return None;
    }
    *runtime = serde_yaml::Value::String(NEW_RUNTIME.to_string());
    let frontmatter = serde_yaml::to_string(&value).ok()?;
    Some(format!("---\n{frontmatter}---\n{body}"))
}

fn migrate_team_personas(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            migrate_team_personas(&path);
            continue;
        }
        if !file_type.is_file()
            || !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".persona.md"))
        {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(updated) = rewrite_persona_runtime(&content) else {
            continue;
        };
        if let Err(error) = std::fs::write(&path, updated) {
            eprintln!(
                "buzz-desktop: goose-default-migration: failed to update {}: {error}",
                path.display()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{migrate_json_file, rewrite_persona_runtime};

    #[test]
    fn migrates_exact_builtin_values_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("managed-agents.json");
        let records = serde_json::json!([
            {
                "name": "Fizz",
                "runtime": "buzz-agent",
                "agent_command": "buzz-agent",
                "agent_command_override": "buzz-agent",
                "mcp_command": "buzz-dev-mcp"
            },
            { "slug": "custom:old", "runtime": "buzz-agent" },
            { "name": "Custom", "agent_command": "/opt/tools/buzz-agent" }
        ]);
        std::fs::write(&path, serde_json::to_vec_pretty(&records).unwrap()).unwrap();

        migrate_json_file(&path);
        let migrated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(migrated[0]["runtime"], "goose");
        assert_eq!(migrated[0]["agent_command"], "goose-acp");
        assert_eq!(migrated[0]["agent_command_override"], "goose-acp");
        assert_eq!(migrated[0]["mcp_command"], "");
        assert_eq!(migrated[1]["runtime"], "goose");
        assert_eq!(migrated[2]["agent_command"], "/opt/tools/buzz-agent");

        let once = std::fs::read_to_string(&path).unwrap();
        migrate_json_file(&path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), once);
    }

    #[test]
    fn rewrites_only_frontmatter_runtime() {
        let content = "---\nname: Fizz\nruntime: buzz-agent\n---\nUse buzz-agent in examples.\n";
        let updated = rewrite_persona_runtime(content).unwrap();
        assert!(updated.contains("runtime: goose"));
        assert!(updated.ends_with("Use buzz-agent in examples.\n"));
        assert!(rewrite_persona_runtime(&updated).is_none());
    }

    #[test]
    fn preserves_other_runtimes() {
        let content = "---\nname: Custom\nruntime: codex\n---\nBody\n";
        assert!(rewrite_persona_runtime(content).is_none());
    }
}
