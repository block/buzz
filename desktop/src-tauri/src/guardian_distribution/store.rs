use crate::managed_agents::storage::atomic_write_json_restricted;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

const RECEIPT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct InstalledReceipt {
    pub schema_version: u32,
    pub component: String,
    pub version: String,
    pub target: String,
    pub manifest_sha256: String,
    pub archive_sha256: String,
    pub binary_sha256: String,
    pub binary_path: String,
    pub source_commit: String,
    pub installed_at: String,
    pub installer_buzz_version: String,
}

impl InstalledReceipt {
    pub(crate) fn write_to(&self, path: &Path) -> Result<(), String> {
        self.validate_fields()?;
        let payload = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("serialize Guardian receipt: {error}"))?;
        atomic_write_json_restricted(path, &payload)
    }

    fn validate_fields(&self) -> Result<(), String> {
        if self.schema_version != RECEIPT_SCHEMA_VERSION || self.component != "numbat" {
            return Err("unsupported Guardian receipt identity".into());
        }
        for digest in [
            &self.manifest_sha256,
            &self.archive_sha256,
            &self.binary_sha256,
        ] {
            if digest.len() != 64
                || !digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err("Guardian receipt digest is not canonical SHA-256".into());
            }
        }
        if self.version.is_empty()
            || self.target.is_empty()
            || self.source_commit.len() != 40
            || self.installed_at.is_empty()
            || self.installer_buzz_version.is_empty()
        {
            return Err("Guardian receipt metadata is incomplete".into());
        }
        safe_relative_binary_path(&self.binary_path)?;
        Ok(())
    }
}

pub(crate) fn load_verified_receipt(
    component_root: &Path,
    receipt_path: &Path,
) -> Result<(InstalledReceipt, PathBuf), String> {
    reject_symlink(receipt_path, "receipt")?;
    let bytes =
        fs::read(receipt_path).map_err(|error| format!("read Guardian receipt: {error}"))?;
    let receipt: InstalledReceipt = serde_json::from_slice(&bytes)
        .map_err(|error| format!("parse Guardian receipt: {error}"))?;
    receipt.validate_fields()?;

    let receipt_dir = receipt_path
        .parent()
        .ok_or("Guardian receipt has no parent directory")?;
    let binary_path = receipt_dir.join(safe_relative_binary_path(&receipt.binary_path)?);
    if !binary_path.starts_with(component_root) {
        return Err("Guardian binary escapes the component store".into());
    }
    reject_symlink(&binary_path, "binary")?;
    let actual = sha256_file(&binary_path)?;
    if actual != receipt.binary_sha256 {
        return Err("Guardian managed binary is tampered".into());
    }
    Ok((receipt, binary_path))
}

fn safe_relative_binary_path(value: &str) -> Result<&Path, String> {
    if value.is_empty() || value.contains('\\') || value.contains('\0') {
        return Err("unsafe Guardian binary path".into());
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("unsafe Guardian binary path".into());
    }
    Ok(path)
}

fn reject_symlink(path: &Path, label: &str) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect Guardian {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("Guardian {label} is not a regular file"));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open Guardian binary: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("read Guardian binary: {error}"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receipt(binary_sha256: String) -> InstalledReceipt {
        InstalledReceipt {
            schema_version: 1,
            component: "numbat".into(),
            version: "0.1.2".into(),
            target: "aarch64-apple-darwin".into(),
            manifest_sha256: "1".repeat(64),
            archive_sha256: "2".repeat(64),
            binary_sha256,
            binary_path: "numbat".into(),
            source_commit: "0e41ad66f5557f412eae330576271f2ee809d3de".into(),
            installed_at: "2026-08-02T00:00:00Z".into(),
            installer_buzz_version: "1.0.0".into(),
        }
    }

    #[test]
    fn receipt_load_rehashes_the_exact_managed_binary() {
        let root = tempfile::tempdir().unwrap();
        let version = root.path().join("versions/0.1.2/aarch64-apple-darwin");
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join("numbat"), b"verified binary").unwrap();
        let digest = sha256_file(&version.join("numbat")).unwrap();
        receipt(digest)
            .write_to(&version.join("receipt.json"))
            .unwrap();

        let (_, binary) =
            load_verified_receipt(root.path(), &version.join("receipt.json")).unwrap();
        assert_eq!(binary, version.join("numbat"));
    }

    #[test]
    fn changed_binary_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let version = root.path().join("versions/0.1.2/aarch64-apple-darwin");
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join("numbat"), b"original").unwrap();
        receipt(sha256_file(&version.join("numbat")).unwrap())
            .write_to(&version.join("receipt.json"))
            .unwrap();
        fs::write(version.join("numbat"), b"replacement").unwrap();

        assert!(
            load_verified_receipt(root.path(), &version.join("receipt.json"))
                .unwrap_err()
                .contains("tampered")
        );
    }

    #[test]
    fn traversal_and_unknown_receipt_fields_are_rejected() {
        let mut value = serde_json::to_value(receipt("3".repeat(64))).unwrap();
        value["binary_path"] = "../numbat".into();
        assert!(serde_json::from_value::<InstalledReceipt>(value.clone())
            .unwrap()
            .validate_fields()
            .is_err());
        value["binary_path"] = "numbat".into();
        value["unexpected"] = true.into();
        assert!(serde_json::from_value::<InstalledReceipt>(value).is_err());
    }
}
