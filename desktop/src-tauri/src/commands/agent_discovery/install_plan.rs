use std::path::Path;

/// Returns the adapter install commands that `install_acp_runtime_blocking` would
/// run for `runtime_id` given a resolved adapter binary at `adapter_path` (or
/// `None` if none was found).
///
/// Returns `None` when no install is needed (adapter is present and current).
/// Returns `Some(cmds)` when the adapter is missing or (for codex) below its
/// minimum supported version.
///
/// For the codex **outdated** case the returned sequence is a two-step
/// reinstall: first uninstall the old `@zed-industries/codex-acp` package
/// (idempotent — exit 0 when absent), then install the new
/// `@agentclientprotocol/codex-acp`. This is required because both packages
/// install a global binary named `codex-acp`, and npm ≥7 refuses to overwrite
/// a bin file owned by a different package with `EEXIST`.
///
/// For the **missing** case the catalog's `adapter_install_commands` are used
/// as-is (no prior package to remove).
///
/// This is a pure planning function: it never spawns a process. Tests use it to
/// assert the correct install command is selected without touching real npm.
pub(super) fn plan_adapter_install<'c>(
    runtime_id: &str,
    adapter_path: Option<&Path>,
    adapter_install_commands: &'c [&'c str],
    adapter_probe_path: Option<&str>,
) -> Option<Vec<&'c str>> {
    match adapter_path {
        // Adapter present and current — no install needed.
        Some(_) if runtime_id != "codex" => None,
        Some(path)
            if !crate::managed_agents::codex_adapter_is_outdated_with_path(
                path,
                adapter_probe_path,
            ) =>
        {
            None
        }
        // Codex adapter is outdated: uninstall the old package first so npm
        // doesn't hit EEXIST on the shared `codex-acp` bin-link, then install.
        Some(_) => Some(vec![
            "npm uninstall -g @zed-industries/codex-acp",
            "npm install -g @agentclientprotocol/codex-acp",
        ]),
        // Adapter missing: use the catalog's install commands directly.
        None => Some(adapter_install_commands.to_vec()),
    }
}

#[cfg(not(windows))]
fn shell_quote_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\"'\"'"))
}

#[cfg(windows)]
fn powershell_quote_arg(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "''"))
}

pub(super) fn goose_update_command(path: &Path) -> String {
    let path = path.display().to_string();
    #[cfg(windows)]
    {
        format!(
            "powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"& {} update\"",
            powershell_quote_arg(&path)
        )
    }
    #[cfg(not(windows))]
    {
        format!("{} update", shell_quote_arg(&path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(windows))]
    #[test]
    fn goose_update_command_shell_quotes_resolved_path() {
        let command = goose_update_command(Path::new("/tmp/Buzz's Goose/goose"));
        assert_eq!(command, "'/tmp/Buzz'\"'\"'s Goose/goose' update");
    }

    #[cfg(windows)]
    #[test]
    fn goose_update_command_uses_native_powershell() {
        let command = goose_update_command(Path::new(r"C:\Users\lifei\Buzz's Goose\goose.exe"));
        assert_eq!(
            command,
            r#"powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "& 'C:\Users\lifei\Buzz''s Goose\goose.exe' update""#
        );
    }
}
