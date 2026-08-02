use std::path::{Path, PathBuf};
use tauri::AppHandle;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct SelectedNumbatBinary {
    pub(super) path: PathBuf,
    pub(super) managed: bool,
}

pub(super) fn select_numbat_binary(
    component_root: &Path,
    launcher: Option<&Path>,
    external: Option<PathBuf>,
) -> Result<Option<SelectedNumbatBinary>, String> {
    if component_root.join("active.json").exists() {
        crate::guardian_distribution::load_active_receipt(component_root)?
            .ok_or("Buzz-managed Numbat activation is missing")?;
        let launcher =
            launcher.ok_or("Buzz-managed Numbat launcher is missing from this Buzz build")?;
        let metadata = std::fs::symlink_metadata(launcher)
            .map_err(|error| format!("inspect Buzz-managed Numbat launcher: {error}"))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err("Buzz-managed Numbat launcher is not a regular file".into());
        }
        return Ok(Some(SelectedNumbatBinary {
            path: launcher.to_owned(),
            managed: true,
        }));
    }
    Ok(external.map(|path| SelectedNumbatBinary {
        path,
        managed: false,
    }))
}

pub(super) fn select_numbat_binary_for_app(
    app: &AppHandle,
) -> Result<Option<SelectedNumbatBinary>, String> {
    let component_root =
        crate::managed_agents::managed_agents_base_dir(app)?.join("numbat/components");
    let launcher = std::env::current_exe().ok().and_then(|path| {
        path.parent().map(|parent| {
            parent.join(format!(
                "buzz-guardian-numbat{}",
                std::env::consts::EXE_SUFFIX
            ))
        })
    });
    select_numbat_binary(
        &component_root,
        launcher.as_deref(),
        crate::managed_agents::resolve_command("numbat"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_numbat_is_used_only_before_managed_activation() {
        let root = tempfile::tempdir().expect("tempdir");
        let external = root.path().join("external-numbat");
        let selected = select_numbat_binary(root.path(), None, Some(external.clone()))
            .expect("selection")
            .expect("external");
        assert_eq!(selected.path, external);
        assert!(!selected.managed);
    }

    #[test]
    fn corrupt_managed_activation_never_falls_back_to_path() {
        let root = tempfile::tempdir().expect("tempdir");
        std::fs::write(root.path().join("active.json"), b"not valid json").expect("active pointer");
        let launcher = root.path().join("buzz-guardian-numbat");
        std::fs::write(&launcher, b"launcher").expect("launcher");

        assert!(select_numbat_binary(
            root.path(),
            Some(&launcher),
            Some(root.path().join("external-numbat")),
        )
        .is_err());
    }
}
