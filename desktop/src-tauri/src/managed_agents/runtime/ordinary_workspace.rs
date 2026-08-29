use std::path::{Path, PathBuf};

pub(super) fn configure_outer_workdir(
    command: &mut std::process::Command,
    trusted: bool,
    pubkey: &str,
) -> Result<(), String> {
    command
        .env_remove("BUZZ_ACP_CHILD_WORKSPACE")
        .env_remove("BUZZ_ACP_CHILD_SCRATCH");
    let workdir = if trusted {
        crate::managed_agents::default_agent_workdir()
    } else {
        Some(prepare_ordinary_agent_scratch(pubkey)?)
    };
    if let Some(workdir) = workdir {
        command.current_dir(&workdir);
        if !trusted {
            // Bind child cwd, sandbox grant, and ACP session metadata to the
            // same private directory. Never grant the whole Buzz Nest.
            command.env("BUZZ_ACP_CHILD_SCRATCH", workdir);
        }
    }
    Ok(())
}

fn prepare_ordinary_agent_scratch(pubkey: &str) -> Result<PathBuf, String> {
    let nest = crate::managed_agents::nest_dir()
        .ok_or_else(|| "cannot resolve Buzz Nest for ordinary agent scratch".to_string())?;
    prepare_ordinary_agent_scratch_at(&nest, pubkey)
}

fn prepare_ordinary_agent_scratch_at(nest: &Path, pubkey: &str) -> Result<PathBuf, String> {
    if pubkey.len() != 64 || !pubkey.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("ordinary agent scratch requires a canonical pubkey".to_string());
    }
    require_real_directory(nest, "Buzz Nest")?;
    let scratch_root = nest.join(".scratch");
    require_real_directory(&scratch_root, "Buzz scratch root")?;
    let managed_root = scratch_root.join("managed-agents");
    ensure_private_directory(&managed_root)?;
    let agent_root = managed_root.join(pubkey.to_ascii_lowercase());
    ensure_private_directory(&agent_root)?;
    agent_root
        .canonicalize()
        .map_err(|error| format!("canonicalize ordinary agent scratch: {error}"))
}

fn require_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} is not a real directory: {}", path.display()));
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), String> {
    match path.symlink_metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(format!(
                "ordinary agent scratch component is not a real directory: {}",
                path.display()
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(path)
                .map_err(|error| format!("create ordinary agent scratch {}: {error}", path.display()))?;
        }
        Err(error) => {
            return Err(format!(
                "inspect ordinary agent scratch {}: {error}",
                path.display()
            ));
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("secure ordinary agent scratch {}: {error}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_one_private_scratch_without_exposing_nest_siblings() {
        let root = std::env::temp_dir().join(format!(
            "buzz-desktop-scratch-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        let nest = root.join(".buzz");
        std::fs::create_dir_all(nest.join(".scratch")).expect("create nest fixture");
        std::fs::write(nest.join("sibling-secret"), b"secret").expect("write sibling canary");
        let pubkey = "ab".repeat(32);
        let scratch = prepare_ordinary_agent_scratch_at(&nest, &pubkey)
            .expect("create private agent scratch");
        assert_eq!(scratch.file_name().and_then(|name| name.to_str()), Some(pubkey.as_str()));
        assert!(!scratch.join("sibling-secret").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                scratch
                    .metadata()
                    .expect("scratch metadata")
                    .permissions()
                    .mode()
                    & 0o7777,
                0o700
            );
        }
        std::fs::remove_dir_all(root).expect("remove scratch fixture");
    }
}
