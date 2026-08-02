use super::{
    activate_receipt, builtin_manifest, builtin_manifest_sha256, fetch_verified_artifact_to,
    inspect_and_extract, load_verified_receipt, InstalledReceipt,
};
use chrono::Utc;
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};

pub(crate) async fn install_current(
    component_root: &Path,
    target: &str,
    buzz_version: &str,
) -> Result<(), String> {
    let manifest = builtin_manifest()?;
    let artifact = manifest.artifact_for(std::env::consts::OS, std::env::consts::ARCH)?;
    create_restricted_directory(component_root, "component store")?;

    let token = uuid::Uuid::new_v4().simple().to_string();
    let archive_path = component_root.join(format!(".download-{token}"));
    let stage_path = component_root.join(format!(".stage-{token}"));
    let final_path = component_root
        .join("versions")
        .join(&manifest.version)
        .join(target);

    let result = async {
        if final_path.exists() {
            let receipt_path = final_path.join("receipt.json");
            load_verified_receipt(component_root, &receipt_path)?;
            activate_receipt(component_root, &receipt_path)?;
            return Ok(());
        }
        let versions_root = component_root.join("versions");
        create_restricted_directory(&versions_root, "version store")?;
        let version_root = versions_root.join(&manifest.version);
        create_restricted_directory(&version_root, "version directory")?;
        let parent = final_path
            .parent()
            .ok_or("Guardian target has no version directory")?;
        ensure_existing_directory(parent, "version directory")?;

        let mut archive = create_private_file(&archive_path)?;
        fetch_verified_artifact_to(artifact, &mut archive).await?;
        archive
            .sync_all()
            .map_err(|error| format!("sync Guardian download: {error}"))?;
        drop(archive);

        let archive = File::open(&archive_path)
            .map_err(|error| format!("open verified Guardian download: {error}"))?;
        inspect_and_extract(
            archive,
            &stage_path,
            artifact,
            &manifest.license,
            manifest.limits,
        )?;
        make_binary_executable(&stage_path.join(&artifact.binary_path))?;

        let receipt = InstalledReceipt {
            schema_version: 1,
            component: manifest.component.clone(),
            version: manifest.version.clone(),
            target: target.to_owned(),
            manifest_sha256: builtin_manifest_sha256(),
            archive_sha256: artifact.archive_sha256.clone(),
            binary_sha256: artifact.binary_sha256.clone(),
            binary_path: artifact.binary_path.clone(),
            source_commit: manifest.source.commit.clone(),
            installed_at: Utc::now().to_rfc3339(),
            installer_buzz_version: buzz_version.to_owned(),
        };
        let staged_receipt = stage_path.join("receipt.json");
        receipt.write_to(&staged_receipt)?;
        load_verified_receipt(component_root, &staged_receipt)?;
        fs::rename(&stage_path, &final_path)
            .map_err(|error| format!("publish Guardian version atomically: {error}"))?;
        let published_receipt = final_path.join("receipt.json");
        load_verified_receipt(component_root, &published_receipt)?;
        activate_receipt(component_root, &published_receipt).map(|_| ())
    }
    .await;

    let _ = fs::remove_file(&archive_path);
    if stage_path.exists() {
        let _ = fs::remove_dir_all(&stage_path);
    }
    result
}

fn create_private_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|error| format!("create private Guardian download: {error}"))
}

fn restrict_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("restrict Guardian directory: {error}"))?;
    }
    Ok(())
}

fn create_restricted_directory(path: &Path, label: &str) -> Result<(), String> {
    if path.exists() {
        ensure_existing_directory(path, label)?;
    } else {
        let parent = path
            .parent()
            .ok_or_else(|| format!("Guardian {label} has no parent directory"))?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("create Guardian {label} parent: {error}"))?;
        ensure_existing_directory(parent, &format!("{label} parent"))?;
        fs::create_dir(path).map_err(|error| format!("create Guardian {label}: {error}"))?;
        ensure_existing_directory(path, label)?;
    }
    restrict_directory(path)
}

fn ensure_existing_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect Guardian {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("Guardian {label} is not a real directory"));
    }
    Ok(())
}

fn make_binary_executable(path: &PathBuf) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("mark Guardian binary executable: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardian_distribution::load_active_receipt;

    #[tokio::test]
    #[ignore = "downloads the pinned upstream release artifact"]
    async fn real_release_installs_verifies_and_activates_atomically() {
        let root = tempfile::tempdir().unwrap();
        install_current(root.path(), "integration-target", "test")
            .await
            .unwrap();
        let (receipt, binary) = load_active_receipt(root.path()).unwrap().unwrap();
        assert_eq!(receipt.version, "0.1.2");
        assert_eq!(receipt.target, "integration-target");
        assert!(binary.is_file());
        assert!(!root.path().join(".stage-orphan").exists());
    }

    #[cfg(unix)]
    #[test]
    fn managed_store_rejects_symlinked_directories() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let component_root = parent.path().join("components");
        symlink(outside.path(), &component_root).unwrap();
        assert!(
            create_restricted_directory(&component_root, "component store")
                .unwrap_err()
                .contains("not a real directory")
        );

        fs::remove_file(&component_root).unwrap();
        create_restricted_directory(&component_root, "component store").unwrap();
        let versions = component_root.join("versions");
        symlink(outside.path(), &versions).unwrap();
        assert!(create_restricted_directory(&versions, "version store")
            .unwrap_err()
            .contains("not a real directory"));
    }
}
