use std::path::Path;

use crate::managed_agents::KnownAcpRuntime;

/// Plan the vendor CLI portion of an explicit runtime setup request.
///
/// Missing CLIs use the catalog installer. An already-installed Goose uses its
/// resolved executable for an optional user-requested update. Other installed
/// CLIs need no setup command.
pub(super) fn plan(runtime: &KnownAcpRuntime) -> Vec<(&'static str, String)> {
    let Some(cli) = runtime.underlying_cli else {
        return Vec::new();
    };
    let cli_path = crate::managed_agents::resolve_command(cli);
    plan_commands(
        runtime.id,
        cli_path.as_deref(),
        runtime.cli_install_commands_for_os(),
    )
}

fn plan_commands(
    runtime_id: &str,
    cli_path: Option<&Path>,
    install_commands: &[&str],
) -> Vec<(&'static str, String)> {
    match cli_path {
        Some(path) if runtime_id == "goose" => {
            vec![("update", goose_update_command(path))]
        }
        Some(_) => Vec::new(),
        None => install_commands
            .iter()
            .map(|command| ("cli", (*command).to_string()))
            .collect(),
    }
}

#[cfg(not(windows))]
fn goose_update_command(path: &Path) -> String {
    let path = path.display().to_string();
    format!("'{}' update", path.replace('\'', "'\"'\"'"))
}

#[cfg(windows)]
fn goose_update_command(path: &Path) -> String {
    let path = path.display().to_string();
    let quoted_path = format!("'{}'", path.replace('\'', "''"));
    format!("powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"& {quoted_path} update\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_goose_uses_catalog_installer() {
        assert_eq!(
            plan_commands("goose", None, &["install goose"]),
            vec![("cli", "install goose".to_string())]
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn installed_goose_updates_resolved_binary() {
        let path = Path::new("/tmp/Buzz's Goose/goose");

        assert_eq!(
            plan_commands("goose", Some(path), &["install goose"]),
            vec![(
                "update",
                "'/tmp/Buzz'\"'\"'s Goose/goose' update".to_string()
            )]
        );
    }

    #[cfg(windows)]
    #[test]
    fn installed_goose_uses_native_powershell() {
        let path = Path::new(r"C:\Users\lifei\Buzz's Goose\goose.exe");

        assert_eq!(
            plan_commands("goose", Some(path), &["install goose"]),
            vec![(
                "update",
                r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& 'C:\Users\lifei\Buzz''s Goose\goose.exe' update""#
                    .to_string()
            )]
        );
    }

    #[test]
    fn installed_non_goose_is_unchanged() {
        let path = Path::new("/usr/local/bin/claude");

        assert!(plan_commands("claude", Some(path), &["install claude"]).is_empty());
    }
}
