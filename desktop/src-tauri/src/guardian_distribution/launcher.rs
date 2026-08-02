use super::activation::load_active_receipt;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

const PROD_BUNDLE_IDENTIFIER: &str = "xyz.block.buzz.app";

pub fn launcher_main() -> Result<i32, String> {
    let root = production_component_root()?;
    let binary = resolve_launcher_binary(&root)?;
    let mut command = Command::new(binary);
    command.args(std::env::args_os().skip(1));

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(format!("launch verified Numbat: {error}"))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| format!("launch verified Numbat: {error}"))?;
        Ok(status.code().unwrap_or(1))
    }
}

fn production_component_root() -> Result<PathBuf, String> {
    let data = dirs::data_dir().ok_or("could not resolve the platform data directory")?;
    Ok(data
        .join(PROD_BUNDLE_IDENTIFIER)
        .join("agents/numbat/components"))
}

fn resolve_launcher_binary(component_root: &Path) -> Result<PathBuf, String> {
    let (_, binary) =
        load_active_receipt(component_root)?.ok_or("Buzz-managed Numbat is not active")?;
    Ok(binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guardian_distribution::{activate_receipt, store::InstalledReceipt};
    use sha2::{Digest, Sha256};
    use std::fs;

    fn install(root: &Path) -> (PathBuf, PathBuf) {
        let dir = root.join("versions/0.1.2/test-target");
        fs::create_dir_all(&dir).unwrap();
        let binary = dir.join("numbat");
        fs::write(&binary, b"verified launcher target").unwrap();
        let receipt = InstalledReceipt {
            schema_version: 1,
            component: "numbat".into(),
            version: "0.1.2".into(),
            target: "test-target".into(),
            manifest_sha256: "1".repeat(64),
            archive_sha256: "2".repeat(64),
            binary_sha256: hex::encode(Sha256::digest(b"verified launcher target")),
            binary_path: "numbat".into(),
            source_commit: "0e41ad66f5557f412eae330576271f2ee809d3de".into(),
            installed_at: "2026-08-02T00:00:00Z".into(),
            installer_buzz_version: "1.0.0".into(),
        };
        let receipt_path = dir.join("receipt.json");
        receipt.write_to(&receipt_path).unwrap();
        (receipt_path, binary)
    }

    #[test]
    fn resolves_only_the_active_verified_binary() {
        let root = tempfile::tempdir().unwrap();
        assert!(resolve_launcher_binary(root.path()).is_err());
        let (receipt, binary) = install(root.path());
        activate_receipt(root.path(), &receipt).unwrap();
        assert_eq!(resolve_launcher_binary(root.path()).unwrap(), binary);
    }

    #[test]
    fn refuses_a_changed_active_binary() {
        let root = tempfile::tempdir().unwrap();
        let (receipt, binary) = install(root.path());
        activate_receipt(root.path(), &receipt).unwrap();
        fs::write(binary, b"changed after activation").unwrap();
        assert!(resolve_launcher_binary(root.path())
            .unwrap_err()
            .contains("tampered"));
    }
}
