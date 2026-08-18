//! Process-wide exclusion for one agent identity on one relay.
//!
//! Relay event deduplication and per-channel in-flight tracking are intentionally
//! local to one harness process. This lease prevents a second local harness from
//! subscribing as the same agent on the same relay and independently processing
//! the same events.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Holds the exclusive OS lock for the lifetime of a harness process.
pub(crate) struct InstanceLease {
    _file: File,
}

#[derive(Debug, Error)]
pub(crate) enum InstanceLeaseError {
    #[error("agent public key must be exactly 64 hexadecimal characters")]
    InvalidPubkey,
    #[error("invalid relay URL for runtime identity: {0}")]
    InvalidRelayUrl(#[from] buzz_core::relay::NormalizeRelayUrlError),
    #[error("could not determine a per-user data directory for the runtime lease")]
    DataDirectoryUnavailable,
    #[error("failed to create runtime lease directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to open runtime lease file {path}: {source}")]
    OpenFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("another buzz-acp process is already active for agent {pubkey} on {relay_url}")]
    AlreadyActive { pubkey: String, relay_url: String },
    #[error("failed to lock runtime lease file {path}: {source}")]
    LockFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl InstanceLease {
    /// Acquire the local process lease for an agent/relay pair without waiting.
    pub(crate) fn acquire(pubkey: &str, relay_url: &str) -> Result<Self, InstanceLeaseError> {
        let base = dirs::data_local_dir().ok_or(InstanceLeaseError::DataDirectoryUnavailable)?;
        Self::acquire_in_dir(
            &base.join("buzz").join("buzz-acp-runtime-leases"),
            pubkey,
            relay_url,
        )
    }

    fn acquire_in_dir(
        directory: &Path,
        pubkey: &str,
        relay_url: &str,
    ) -> Result<Self, InstanceLeaseError> {
        let pubkey = canonical_pubkey(pubkey)?;
        let relay_url = buzz_core::relay::normalize_relay_url(relay_url)?;
        create_lease_directory(directory)?;

        let relay_hash = hex::encode(Sha256::digest(relay_url.as_bytes()));
        let path = directory.join(format!("{pubkey}__{relay_hash}.lock"));
        let file = open_lease_file(&path)?;

        match file.try_lock_exclusive() {
            Ok(()) => Ok(Self { _file: file }),
            Err(source) if lock_was_contended(&source) => {
                Err(InstanceLeaseError::AlreadyActive { pubkey, relay_url })
            }
            Err(source) => Err(InstanceLeaseError::LockFile { path, source }),
        }
    }
}

fn lock_was_contended(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::WouldBlock {
        return true;
    }

    match (
        error.raw_os_error(),
        fs2::lock_contended_error().raw_os_error(),
    ) {
        (Some(actual), Some(expected)) => actual == expected,
        _ => false,
    }
}

fn canonical_pubkey(pubkey: &str) -> Result<String, InstanceLeaseError> {
    let trimmed = pubkey.trim();
    if trimmed.len() != 64 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(InstanceLeaseError::InvalidPubkey);
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn create_lease_directory(path: &Path) -> Result<(), InstanceLeaseError> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|source| InstanceLeaseError::CreateDirectory {
            path: path.to_path_buf(),
            source,
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            InstanceLeaseError::CreateDirectory {
                path: path.to_path_buf(),
                source,
            }
        })?;
    }

    Ok(())
}

fn open_lease_file(path: &Path) -> Result<File, InstanceLeaseError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|source| InstanceLeaseError::OpenFile {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};

    const PUBKEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const PUBKEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const CHILD_DIRECTORY_ENV: &str = "BUZZ_ACP_LEASE_TEST_CHILD_DIRECTORY";
    const CHILD_READY_ENV: &str = "BUZZ_ACP_LEASE_TEST_CHILD_READY";
    const CHILD_RELEASE_ENV: &str = "BUZZ_ACP_LEASE_TEST_CHILD_RELEASE";

    #[test]
    fn same_pair_contends_without_waiting() {
        let directory = tempfile::tempdir().expect("temp directory");
        let _first =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example")
                .expect("first lease");

        let second =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example");

        assert!(matches!(
            second,
            Err(InstanceLeaseError::AlreadyActive { .. })
        ));
    }

    #[test]
    fn subprocess_lease_holder() {
        let Some(directory) = std::env::var_os(CHILD_DIRECTORY_ENV) else {
            return;
        };
        let ready = PathBuf::from(
            std::env::var_os(CHILD_READY_ENV).expect("child ready marker path must be set"),
        );
        let release = PathBuf::from(
            std::env::var_os(CHILD_RELEASE_ENV).expect("child release marker path must be set"),
        );

        let _lease =
            InstanceLease::acquire_in_dir(Path::new(&directory), PUBKEY_A, "wss://relay.example")
                .expect("child lease");
        fs::write(&ready, b"ready").expect("write child ready marker");

        let deadline = Instant::now() + Duration::from_secs(10);
        while !release.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(release.exists(), "parent did not release child lease test");
    }

    #[test]
    fn same_pair_contends_across_processes() {
        let directory = tempfile::tempdir().expect("temp directory");
        let ready = directory.path().join("child-ready");
        let release = directory.path().join("child-release");
        let mut child = spawn_lease_holder(directory.path(), &ready, &release);
        wait_for_ready(&mut child, &ready);

        let second =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example");
        let contended = matches!(second, Err(InstanceLeaseError::AlreadyActive { .. }));

        fs::write(&release, b"release").expect("release child lease");
        let status = child.wait().expect("wait for lease-holder subprocess");
        assert!(status.success(), "lease-holder subprocess failed: {status}");
        assert!(contended, "second process unexpectedly acquired the lease");
        InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example")
            .expect("lease after subprocess exit");
    }

    #[test]
    fn crashed_process_releases_pair() {
        let directory = tempfile::tempdir().expect("temp directory");
        let ready = directory.path().join("child-ready");
        let release = directory.path().join("child-release");
        let mut child = spawn_lease_holder(directory.path(), &ready, &release);
        wait_for_ready(&mut child, &ready);

        child.kill().expect("terminate lease-holder subprocess");
        child.wait().expect("reap lease-holder subprocess");

        InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example")
            .expect("lease after subprocess termination");
    }

    #[test]
    fn released_pair_can_be_reacquired() {
        let directory = tempfile::tempdir().expect("temp directory");
        let first =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example")
                .expect("first lease");
        drop(first);

        InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example")
            .expect("lease after release");
    }

    #[test]
    fn normalized_relay_spellings_share_one_lease() {
        let directory = tempfile::tempdir().expect("temp directory");
        let _first =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, " WSS://Relay.Example:443/ ")
                .expect("first lease");

        let second =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example");

        assert!(matches!(
            second,
            Err(InstanceLeaseError::AlreadyActive { .. })
        ));
    }

    #[test]
    fn independent_agent_relay_pairs_can_coexist() {
        let directory = tempfile::tempdir().expect("temp directory");
        let _first =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://relay.example")
                .expect("first pair");
        let _different_agent =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_B, "wss://relay.example")
                .expect("different agent");
        let _different_relay =
            InstanceLease::acquire_in_dir(directory.path(), PUBKEY_A, "wss://other.example")
                .expect("different relay");
    }

    fn spawn_lease_holder(directory: &Path, ready: &Path, release: &Path) -> Child {
        Command::new(std::env::current_exe().expect("current test executable"))
            .arg("--exact")
            .arg("instance_lease::tests::subprocess_lease_holder")
            .arg("--nocapture")
            .env(CHILD_DIRECTORY_ENV, directory)
            .env(CHILD_READY_ENV, ready)
            .env(CHILD_RELEASE_ENV, release)
            .spawn()
            .expect("spawn lease-holder subprocess")
    }

    fn wait_for_ready(child: &mut Child, ready: &Path) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !ready.exists() && Instant::now() < deadline {
            if let Some(status) = child.try_wait().expect("inspect child status") {
                panic!("lease-holder subprocess exited early with {status}");
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            ready.exists(),
            "lease-holder subprocess did not become ready"
        );
    }
}
