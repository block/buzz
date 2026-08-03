pub(super) fn validate_managed_adapter_descriptor(
    command: &str,
    args: &[String],
) -> Result<(), String> {
    let canonical_command = crate::managed_agents::default_agent_command();
    if command != &canonical_command || !args.is_empty() {
        return Err(
            "unsupported_managed_adapter: durable managed mode requires the canonical bundled buzz-agent command with default arguments"
                .into(),
        );
    }
    Ok(())
}

pub(super) fn is_bundled_sibling(
    resolved: &std::path::Path,
    desktop_executable: &std::path::Path,
) -> bool {
    let Some(resolved_parent) = resolved.parent() else {
        return false;
    };
    let Some(desktop_parent) = desktop_executable.parent() else {
        return false;
    };
    resolved_parent == desktop_parent
        || (desktop_parent
            .file_name()
            .is_some_and(|name| name == "deps")
            && desktop_parent.parent() == Some(resolved_parent))
}

pub(super) fn resolve_canonical_bundled_executable(
    command: &str,
    executable_name: &str,
) -> Result<std::path::PathBuf, String> {
    let unsupported = || {
        format!(
            "unsupported_managed_adapter: canonical bundled {executable_name} executable was not found"
        )
    };
    let resolved = super::resolve_command(command)
        .and_then(|path| std::fs::canonicalize(path).ok())
        .ok_or_else(&unsupported)?;
    let expected_file_name = format!("{executable_name}{}", std::env::consts::EXE_SUFFIX);
    if resolved.file_name() != Some(std::ffi::OsStr::new(&expected_file_name)) {
        return Err(unsupported());
    }
    let desktop_executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| unsupported())?;
    if !is_bundled_sibling(&resolved, &desktop_executable) {
        return Err(unsupported());
    }
    Ok(resolved)
}

pub(super) fn resolve_canonical_bundled_buzz_agent() -> Result<std::path::PathBuf, String> {
    resolve_canonical_bundled_executable(
        &crate::managed_agents::default_agent_command(),
        "buzz-agent",
    )
}
