use super::store::{load_verified_receipt, InstalledReceipt};
use crate::managed_agents::storage::atomic_write_json_restricted;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

const ACTIVE_SCHEMA_VERSION: u32 = 1;
const ACTIVE_FILE: &str = "active.json";
const JOURNAL_FILE: &str = "activation-journal.ndjson";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ActivePointer {
    schema_version: u32,
    generation: u64,
    receipt_path: String,
    receipt_sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum JournalEntry {
    Prepared {
        generation: u64,
        next: ActivePointer,
        previous: Option<ActivePointer>,
    },
    Committed {
        generation: u64,
    },
    DeactivationPrepared {
        generation: u64,
        previous: ActivePointer,
    },
    Deactivated {
        generation: u64,
    },
    Recovered {
        generation: u64,
    },
}

pub(crate) fn activate_receipt(
    component_root: &Path,
    receipt_path: &Path,
) -> Result<InstalledReceipt, String> {
    let (receipt, _) = load_verified_receipt(component_root, receipt_path)?;
    let relative = receipt_path
        .strip_prefix(component_root)
        .map_err(|_| "Guardian receipt is outside the component store")?;
    let relative = safe_relative_path(relative)?;
    let previous = read_active_pointer(component_root)?.map(|(pointer, _)| pointer);
    let generation = previous
        .as_ref()
        .map_or(1, |pointer| pointer.generation.saturating_add(1));
    if generation == u64::MAX {
        return Err("Guardian activation generation is exhausted".into());
    }
    let next = ActivePointer {
        schema_version: ACTIVE_SCHEMA_VERSION,
        generation,
        receipt_path: relative,
        receipt_sha256: sha256_file(receipt_path)?,
    };
    append_journal(
        component_root,
        &JournalEntry::Prepared {
            generation,
            next: next.clone(),
            previous,
        },
    )?;
    write_active_pointer(component_root, &next)?;
    append_journal(component_root, &JournalEntry::Committed { generation })?;
    Ok(receipt)
}

pub(crate) fn load_active_receipt(
    component_root: &Path,
) -> Result<Option<(InstalledReceipt, PathBuf)>, String> {
    let Some((pointer, receipt_path)) = read_active_pointer(component_root)? else {
        return Ok(None);
    };
    if sha256_file(&receipt_path)? != pointer.receipt_sha256 {
        return Err("Guardian active receipt is tampered".into());
    }
    load_verified_receipt(component_root, &receipt_path).map(Some)
}

pub(crate) fn recover_activation(component_root: &Path) -> Result<bool, String> {
    let entries = read_journal(component_root)?;
    let (generation, previous) = match entries.last() {
        Some(JournalEntry::Prepared {
            generation,
            previous,
            ..
        }) => (*generation, previous.as_ref()),
        Some(JournalEntry::DeactivationPrepared {
            generation,
            previous,
        }) => (*generation, Some(previous)),
        _ => return Ok(false),
    };

    match previous {
        Some(pointer) => write_active_pointer(component_root, pointer)?,
        None => {
            let active = component_root.join(ACTIVE_FILE);
            if active.exists() {
                fs::remove_file(&active)
                    .map_err(|error| format!("remove incomplete Guardian activation: {error}"))?;
            }
        }
    }
    append_journal(component_root, &JournalEntry::Recovered { generation })?;
    Ok(true)
}

pub(crate) fn deactivate(component_root: &Path) -> Result<bool, String> {
    let Some((previous, _)) = read_active_pointer(component_root)? else {
        return Ok(false);
    };
    let generation = previous
        .generation
        .checked_add(1)
        .ok_or("Guardian activation generation is exhausted")?;
    append_journal(
        component_root,
        &JournalEntry::DeactivationPrepared {
            generation,
            previous,
        },
    )?;
    fs::remove_file(component_root.join(ACTIVE_FILE))
        .map_err(|error| format!("deactivate Guardian Numbat: {error}"))?;
    append_journal(component_root, &JournalEntry::Deactivated { generation })?;
    Ok(true)
}

pub(crate) fn rollback(component_root: &Path) -> Result<InstalledReceipt, String> {
    let Some((current, _)) = read_active_pointer(component_root)? else {
        return Err("Guardian Numbat is not active".into());
    };
    let previous = read_journal(component_root)?
        .into_iter()
        .rev()
        .find_map(|entry| match entry {
            JournalEntry::Prepared {
                generation,
                previous: Some(previous),
                ..
            } if generation == current.generation => Some(previous),
            _ => None,
        })
        .ok_or("No previous Guardian Numbat version is available")?;
    let receipt = component_root.join(safe_relative_path(Path::new(&previous.receipt_path))?);
    activate_receipt(component_root, &receipt)
}

pub(crate) fn rollback_available(component_root: &Path) -> Result<bool, String> {
    let Some((current, _)) = read_active_pointer(component_root)? else {
        return Ok(false);
    };
    let previous = read_journal(component_root)?
        .into_iter()
        .rev()
        .find_map(|entry| match entry {
            JournalEntry::Prepared {
                generation,
                previous: Some(previous),
                ..
            } if generation == current.generation => Some(previous),
            _ => None,
        });
    let Some(previous) = previous else {
        return Ok(false);
    };
    let receipt = component_root.join(safe_relative_path(Path::new(&previous.receipt_path))?);
    load_verified_receipt(component_root, &receipt).map(|_| true)
}

pub(crate) fn prune_superseded_versions(component_root: &Path) -> Result<usize, String> {
    let Some((active, active_receipt)) = read_active_pointer(component_root)? else {
        return Ok(0);
    };
    load_verified_receipt(component_root, &active_receipt)?;

    let mut retained = HashSet::from([active_receipt]);
    if let Some(previous) = previous_pointer_for_generation(component_root, active.generation)? {
        let receipt = component_root.join(safe_relative_path(Path::new(&previous.receipt_path))?);
        load_verified_receipt(component_root, &receipt)?;
        retained.insert(receipt);
    }

    let versions_root = component_root.join("versions");
    if !versions_root.exists() {
        return Ok(0);
    }
    ensure_real_directory(&versions_root, "version store")?;

    let mut removed = 0usize;
    for version_entry in read_real_directories(&versions_root, "version")? {
        for target_entry in read_real_directories(&version_entry, "target")? {
            let receipt = target_entry.join("receipt.json");
            if retained.contains(&receipt) {
                continue;
            }
            load_verified_receipt(component_root, &receipt)?;
            fs::remove_dir_all(&target_entry)
                .map_err(|error| format!("remove superseded Guardian version: {error}"))?;
            removed = removed.saturating_add(1);
        }
        if fs::read_dir(&version_entry)
            .map_err(|error| format!("inspect Guardian version directory: {error}"))?
            .next()
            .is_none()
        {
            fs::remove_dir(&version_entry)
                .map_err(|error| format!("remove empty Guardian version directory: {error}"))?;
        }
    }
    Ok(removed)
}

fn previous_pointer_for_generation(
    component_root: &Path,
    generation: u64,
) -> Result<Option<ActivePointer>, String> {
    Ok(read_journal(component_root)?
        .into_iter()
        .rev()
        .find_map(|entry| match entry {
            JournalEntry::Prepared {
                generation: prepared_generation,
                previous,
                ..
            } if prepared_generation == generation => previous,
            _ => None,
        }))
}

fn read_real_directories(root: &Path, label: &str) -> Result<Vec<PathBuf>, String> {
    let mut directories = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|error| format!("read Guardian {label} store: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read Guardian {label} entry: {error}"))?;
        let path = entry.path();
        ensure_real_directory(&path, &format!("{label} entry"))?;
        directories.push(path);
    }
    Ok(directories)
}

fn ensure_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect Guardian {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("Guardian {label} is not a real directory"));
    }
    Ok(())
}

pub(crate) fn uninstall_active(component_root: &Path) -> Result<bool, String> {
    let Some((_, receipt_path)) = read_active_pointer(component_root)? else {
        return Ok(false);
    };
    load_verified_receipt(component_root, &receipt_path)?;
    let version_dir = receipt_path
        .parent()
        .ok_or("Guardian receipt has no version directory")?;
    let versions_root = component_root.join("versions");
    if !version_dir.starts_with(&versions_root) || version_dir == versions_root {
        return Err("Guardian uninstall target is outside the version store".into());
    }
    deactivate(component_root)?;
    fs::remove_dir_all(version_dir)
        .map_err(|error| format!("remove Guardian Numbat version: {error}"))?;
    Ok(true)
}

fn read_active_pointer(component_root: &Path) -> Result<Option<(ActivePointer, PathBuf)>, String> {
    let path = component_root.join(ACTIVE_FILE);
    if !path.exists() {
        return Ok(None);
    }
    reject_non_regular(&path, "active pointer")?;
    let payload =
        fs::read(&path).map_err(|error| format!("read Guardian active pointer: {error}"))?;
    let pointer: ActivePointer = serde_json::from_slice(&payload)
        .map_err(|error| format!("parse Guardian active pointer: {error}"))?;
    pointer.validate()?;
    let receipt_path = component_root.join(safe_relative_path(Path::new(&pointer.receipt_path))?);
    Ok(Some((pointer, receipt_path)))
}

fn write_active_pointer(component_root: &Path, pointer: &ActivePointer) -> Result<(), String> {
    pointer.validate()?;
    fs::create_dir_all(component_root)
        .map_err(|error| format!("create Guardian component root: {error}"))?;
    let payload = serde_json::to_vec_pretty(pointer)
        .map_err(|error| format!("serialize Guardian active pointer: {error}"))?;
    atomic_write_json_restricted(&component_root.join(ACTIVE_FILE), &payload)
}

impl ActivePointer {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != ACTIVE_SCHEMA_VERSION || self.generation == 0 {
            return Err("unsupported Guardian active pointer".into());
        }
        safe_relative_path(Path::new(&self.receipt_path))?;
        validate_digest(&self.receipt_sha256)
    }
}

fn append_journal(component_root: &Path, entry: &JournalEntry) -> Result<(), String> {
    fs::create_dir_all(component_root)
        .map_err(|error| format!("create Guardian component root: {error}"))?;
    let path = component_root.join(JOURNAL_FILE);
    if path.exists() {
        reject_non_regular(&path, "activation journal")?;
    }
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("open Guardian activation journal: {error}"))?;
    serde_json::to_writer(&mut file, entry)
        .map_err(|error| format!("serialize Guardian activation journal: {error}"))?;
    file.write_all(b"\n")
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("persist Guardian activation journal: {error}"))
}

fn read_journal(component_root: &Path) -> Result<Vec<JournalEntry>, String> {
    let path = component_root.join(JOURNAL_FILE);
    if !path.exists() {
        return Ok(Vec::new());
    }
    reject_non_regular(&path, "activation journal")?;
    let payload = fs::read_to_string(&path)
        .map_err(|error| format!("read Guardian activation journal: {error}"))?;
    payload
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|error| format!("parse Guardian activation journal: {error}"))
        })
        .collect()
}

fn safe_relative_path(path: &Path) -> Result<String, String> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("unsafe Guardian receipt path".into());
    }
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .filter(|value| {
                    !value.is_empty()
                        && !value.contains('/')
                        && !value.contains('\\')
                        && !value.contains('\0')
                })
                .ok_or_else(|| "unsafe Guardian receipt path".to_owned()),
            _ => Err("unsafe Guardian receipt path".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|components| components.join("/"))
}

fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err("Guardian receipt digest is not canonical SHA-256".into())
    }
}

fn reject_non_regular(path: &Path, label: &str) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("inspect Guardian {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!("Guardian {label} is not a regular file"));
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|error| format!("open Guardian file: {error}"))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("read Guardian file: {error}"))?;
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

    fn installed(root: &Path, version: &str, body: &[u8]) -> PathBuf {
        let dir = root.join(format!("versions/{version}/aarch64-apple-darwin"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("numbat"), body).unwrap();
        let receipt = InstalledReceipt {
            schema_version: 1,
            component: "numbat".into(),
            version: version.into(),
            target: "aarch64-apple-darwin".into(),
            manifest_sha256: "1".repeat(64),
            archive_sha256: "2".repeat(64),
            binary_sha256: sha256_file(&dir.join("numbat")).unwrap(),
            binary_path: "numbat".into(),
            source_commit: "0e41ad66f5557f412eae330576271f2ee809d3de".into(),
            installed_at: "2026-08-02T00:00:00Z".into(),
            installer_buzz_version: "1.0.0".into(),
        };
        let path = dir.join("receipt.json");
        receipt.write_to(&path).unwrap();
        path
    }

    #[test]
    fn activation_commits_verified_receipt_and_increments_generation() {
        let root = tempfile::tempdir().unwrap();
        let first = installed(root.path(), "0.1.1", b"first");
        activate_receipt(root.path(), &first).unwrap();
        let second = installed(root.path(), "0.1.2", b"second");
        activate_receipt(root.path(), &second).unwrap();

        let (receipt, binary) = load_active_receipt(root.path()).unwrap().unwrap();
        assert_eq!(receipt.version, "0.1.2");
        assert_eq!(fs::read(binary).unwrap(), b"second");
        let (pointer, _) = read_active_pointer(root.path()).unwrap().unwrap();
        assert_eq!(pointer.generation, 2);
        assert!(matches!(
            read_journal(root.path()).unwrap().last(),
            Some(JournalEntry::Committed { generation: 2 })
        ));
    }

    #[test]
    fn receipt_or_binary_tampering_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let receipt = installed(root.path(), "0.1.2", b"verified");
        activate_receipt(root.path(), &receipt).unwrap();
        fs::write(receipt.parent().unwrap().join("numbat"), b"changed").unwrap();
        assert!(load_active_receipt(root.path())
            .unwrap_err()
            .contains("tampered"));
    }

    #[test]
    fn rollback_reactivates_the_previous_verified_version() {
        let root = tempfile::tempdir().unwrap();
        let first = installed(root.path(), "0.1.1", b"first");
        activate_receipt(root.path(), &first).unwrap();
        let second = installed(root.path(), "0.1.2", b"second");
        activate_receipt(root.path(), &second).unwrap();

        assert!(rollback_available(root.path()).unwrap());

        let receipt = rollback(root.path()).unwrap();
        assert_eq!(receipt.version, "0.1.1");
        let (active, binary) = load_active_receipt(root.path()).unwrap().unwrap();
        assert_eq!(active.version, "0.1.1");
        assert_eq!(fs::read(binary).unwrap(), b"first");
        assert_eq!(
            read_active_pointer(root.path())
                .unwrap()
                .unwrap()
                .0
                .generation,
            3
        );
    }

    #[test]
    fn rollback_availability_requires_a_verified_previous_receipt() {
        let root = tempfile::tempdir().unwrap();
        let first = installed(root.path(), "0.1.1", b"first");
        activate_receipt(root.path(), &first).unwrap();
        assert!(!rollback_available(root.path()).unwrap());
        let second = installed(root.path(), "0.1.2", b"second");
        activate_receipt(root.path(), &second).unwrap();
        fs::write(first.parent().unwrap().join("numbat"), b"tampered").unwrap();
        assert!(rollback_available(root.path()).is_err());
    }

    #[test]
    fn deactivation_is_recoverable_and_uninstall_removes_only_active_version() {
        let root = tempfile::tempdir().unwrap();
        let receipt = installed(root.path(), "0.1.2", b"active");
        activate_receipt(root.path(), &receipt).unwrap();
        let (previous, _) = read_active_pointer(root.path()).unwrap().unwrap();
        append_journal(
            root.path(),
            &JournalEntry::DeactivationPrepared {
                generation: 2,
                previous,
            },
        )
        .unwrap();
        fs::remove_file(root.path().join(ACTIVE_FILE)).unwrap();
        assert!(recover_activation(root.path()).unwrap());
        assert!(load_active_receipt(root.path()).unwrap().is_some());

        assert!(uninstall_active(root.path()).unwrap());
        assert!(load_active_receipt(root.path()).unwrap().is_none());
        assert!(!receipt.parent().unwrap().exists());
        assert!(!uninstall_active(root.path()).unwrap());
    }

    #[test]
    fn recovery_restores_previous_pointer_after_interrupted_activation() {
        let root = tempfile::tempdir().unwrap();
        let first = installed(root.path(), "0.1.1", b"first");
        activate_receipt(root.path(), &first).unwrap();
        let (previous, _) = read_active_pointer(root.path()).unwrap().unwrap();
        let second = installed(root.path(), "0.1.2", b"second");
        let next = ActivePointer {
            schema_version: 1,
            generation: 2,
            receipt_path: safe_relative_path(second.strip_prefix(root.path()).unwrap()).unwrap(),
            receipt_sha256: sha256_file(&second).unwrap(),
        };
        append_journal(
            root.path(),
            &JournalEntry::Prepared {
                generation: 2,
                next: next.clone(),
                previous: Some(previous),
            },
        )
        .unwrap();
        write_active_pointer(root.path(), &next).unwrap();

        assert!(recover_activation(root.path()).unwrap());
        assert_eq!(
            load_active_receipt(root.path()).unwrap().unwrap().0.version,
            "0.1.1"
        );
        assert!(!recover_activation(root.path()).unwrap());
    }

    #[test]
    fn pruning_keeps_only_active_and_last_known_good_versions() {
        let root = tempfile::tempdir().unwrap();
        let first = installed(root.path(), "0.1.0", b"first");
        activate_receipt(root.path(), &first).unwrap();
        let second = installed(root.path(), "0.1.1", b"second");
        activate_receipt(root.path(), &second).unwrap();
        let third = installed(root.path(), "0.1.2", b"third");
        activate_receipt(root.path(), &third).unwrap();

        assert_eq!(prune_superseded_versions(root.path()).unwrap(), 1);
        assert!(!first.parent().unwrap().exists());
        assert!(second.parent().unwrap().exists());
        assert!(third.parent().unwrap().exists());
        assert!(rollback_available(root.path()).unwrap());
        assert_eq!(rollback(root.path()).unwrap().version, "0.1.1");
    }

    #[cfg(unix)]
    #[test]
    fn pruning_rejects_symlinked_version_entries() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let receipt = installed(root.path(), "0.1.2", b"active");
        activate_receipt(root.path(), &receipt).unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("versions/0.1.1")).unwrap();

        assert!(prune_superseded_versions(root.path())
            .unwrap_err()
            .contains("not a real directory"));
        assert!(outside.path().exists());
    }
}
