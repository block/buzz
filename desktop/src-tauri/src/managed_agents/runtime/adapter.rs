pub(super) fn validate_managed_adapter_descriptor(
    command: &str,
    args: &[String],
) -> Result<(), String> {
    let canonical_command = crate::managed_agents::default_agent_command();
    if command != canonical_command || !args.is_empty() {
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
pub(super) fn bundled_sibling_candidate(
    desktop_executable: &std::path::Path,
    executable_name: &str,
) -> Option<std::path::PathBuf> {
    let desktop_parent = desktop_executable.parent()?;
    let bundle_directory = if desktop_parent
        .file_name()
        .is_some_and(|name| name == "deps")
    {
        desktop_parent.parent()?
    } else {
        desktop_parent
    };
    Some(bundle_directory.join(format!("{executable_name}{}", std::env::consts::EXE_SUFFIX)))
}
pub(super) fn canonical_executable(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let link_metadata = std::fs::symlink_metadata(path).ok()?;
    if link_metadata.file_type().is_symlink() {
        return None;
    }
    let canonical = std::fs::canonicalize(path).ok()?;
    let metadata = std::fs::metadata(&canonical).ok()?;
    if !metadata.is_file() || metadata.len() == 0 {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    Some(canonical)
}

#[cfg(debug_assertions)]
fn debug_workspace_candidate(executable_name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug")
        .join(format!("{executable_name}{}", std::env::consts::EXE_SUFFIX))
}

#[cfg(debug_assertions)]
fn is_debug_workspace_binary(resolved: &std::path::Path, executable_name: &str) -> bool {
    canonical_executable(&debug_workspace_candidate(executable_name)).as_deref() == Some(resolved)
}

#[cfg(not(debug_assertions))]
fn is_debug_workspace_binary(_resolved: &std::path::Path, _executable_name: &str) -> bool {
    false
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
    let expected_file_name = format!("{executable_name}{}", std::env::consts::EXE_SUFFIX);
    let desktop_executable = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| unsupported())?;
    let resolved = bundled_sibling_candidate(&desktop_executable, executable_name)
        .and_then(|path| canonical_executable(&path))
        .or_else(|| {
            #[cfg(debug_assertions)]
            {
                canonical_executable(&debug_workspace_candidate(executable_name))
            }
            #[cfg(not(debug_assertions))]
            {
                None
            }
        })
        .or_else(|| super::resolve_command(command).and_then(|path| canonical_executable(&path)))
        .ok_or_else(&unsupported)?;
    if resolved.file_name() != Some(std::ffi::OsStr::new(&expected_file_name)) {
        return Err(unsupported());
    }
    if !is_bundled_sibling(&resolved, &desktop_executable)
        && !is_debug_workspace_binary(&resolved, executable_name)
    {
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
