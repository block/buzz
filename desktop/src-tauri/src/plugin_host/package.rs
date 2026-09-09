//! Filesystem package staging, layout limits, and digest verification.
//!
//! Staging is the only validation entry point. On unix, [`unix_fs::copy_validated_tree`]
//! walks the source directory through descriptor-relative, `O_NOFOLLOW` opens
//! (`nix::dir::Dir` / `nix::fcntl::openat`): every directory and file is
//! opened exactly once by file descriptor, its layout limit and (for files)
//! byte budget are enforced from that same open descriptor, and the frozen
//! bytes are copied from it. A symlink swapped in after the enumerating
//! `readdir` call but before the entry is opened is rejected (`ELOOP`)
//! instead of silently followed, and a file grown after its length is read
//! cannot make more bytes land in the staged copy than were budgeted,
//! because the copy reads are bounded by that same length
//! (see `installs_and_resolves_browser_package` and the mutation-during-copy
//! regression in `package_tests.rs`).

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::plugin_host::manifest;
use crate::plugin_host::types::{InstallReject, Manifest, StagedPackage};

/// Maximum number of regular files a package may contain.
const MAX_FILES: usize = 16;
/// Maximum total size in bytes of every regular file in a package.
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum number of files and directories combined (excluding the package
/// root itself), independent of the file-only [`MAX_FILES`] cap.
const MAX_TOTAL_ENTRIES: usize = 256;
/// Maximum nesting depth of any directory in the package.
const MAX_DIRECTORY_LEVELS: usize = 16;
const MANIFEST_FILE_NAME: &str = "manifest.json";

/// Removes a staged directory unless disarmed, so any error during staging
/// — copy failure, invalid manifest, digest mismatch — leaves no partial
/// directory behind. Owns the path (rather than borrowing it) so the
/// `StagedPackage` returned on success can still move `staged_path` out of
/// the function that declared this guard. Owns `trusted_base` too, so the
/// cleanup performed on drop goes through the same ancestry validation as
/// every other owned-storage removal (see [`remove_owned_directory`]).
struct StagingCleanup {
    trusted_base: PathBuf,
    path: PathBuf,
    armed: bool,
}

impl Drop for StagingCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_owned_directory(&self.trusted_base, &self.path);
        }
    }
}

/// Copies `directory` into a fresh directory under `staging_root`, then
/// validates its layout, manifest, and target executable digest against the
/// frozen copy. Any failure after the directory is created removes it.
///
/// `trusted_base` is the plugin host's own configuration root (the directory
/// containing both `plugins/` and `staging/`); `staging_root` must be that
/// root itself or a descendant of it, with no symlink on the path between
/// them. See [`validate_owned_descendant`] for why that matters even though
/// `staging_root` itself might not be a symlink.
pub fn stage(
    trusted_base: &Path,
    directory: &Path,
    staging_root: &Path,
) -> Result<StagedPackage, InstallReject> {
    ensure_owned_directory(trusted_base, staging_root)
        .map_err(|error| InstallReject::InvalidPackage(format!("prepare staging root: {error}")))?;
    let staged_path = staging_root.join(uuid::Uuid::new_v4().to_string());
    let mut cleanup = StagingCleanup {
        trusted_base: trusted_base.to_path_buf(),
        path: staged_path.clone(),
        armed: true,
    };

    #[cfg(unix)]
    unix_fs::copy_validated_tree(directory, &staged_path)?;
    #[cfg(not(unix))]
    fallback_fs::copy_validated_tree(directory, &staged_path)?;

    let manifest_path = staged_path.join(MANIFEST_FILE_NAME);
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| InstallReject::InvalidPackage(format!("read manifest: {error}")))?;
    let manifest = manifest::parse_and_validate(&manifest_bytes)?;
    let executable_sha256 = verify_target_executable(&manifest, &staged_path)?;

    cleanup.armed = false;
    Ok(StagedPackage {
        manifest,
        executable_sha256,
        staged_path,
    })
}

/// Re-verifies a staged package's target executable digest against its
/// frozen bytes. Called immediately before commit so nothing can substitute
/// the executable between staging and commit.
pub fn reverify(staged: &StagedPackage) -> Result<(), InstallReject> {
    let actual = verify_target_executable(&staged.manifest, &staged.staged_path)?;
    if actual != staged.executable_sha256 {
        return Err(InstallReject::DigestMismatch);
    }
    Ok(())
}

/// Digest naming the plugin's committed package directory, computed from the
/// staged (frozen) manifest bytes rather than the caller-supplied source.
pub fn package_digest(staged: &StagedPackage) -> Result<String, InstallReject> {
    let manifest_path = staged.staged_path.join(MANIFEST_FILE_NAME);
    let bytes = fs::read(&manifest_path)
        .map_err(|error| InstallReject::InvalidPackage(format!("read staged manifest: {error}")))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

/// Atomically moves a staged directory to its final committed location.
///
/// `trusted_base` is the plugin host's own configuration root; `final_path`'s
/// parent must be that root or a descendant of it, with no symlink on the
/// path between them (see [`validate_owned_descendant`]).
pub fn finalize(
    trusted_base: &Path,
    staged_path: &Path,
    final_path: &Path,
) -> Result<(), InstallReject> {
    if let Some(parent) = final_path.parent() {
        ensure_owned_directory(trusted_base, parent).map_err(|error| {
            InstallReject::InvalidPackage(format!("prepare plugin directory: {error}"))
        })?;
    }
    if fs::symlink_metadata(final_path).is_ok() {
        return Err(InstallReject::InvalidPackage(
            "plugin package destination already exists".into(),
        ));
    }
    fs::rename(staged_path, final_path)
        .map_err(|error| InstallReject::InvalidPackage(format!("commit package: {error}")))
}

/// Removes a directory this host owns, refusing to act through a symlink at
/// its own root or anywhere on the path above it inside owned storage (see
/// [`validate_owned_descendant`]). Checking only `path`'s own root would miss
/// a symlink planted higher up the owned tree — for example the shared
/// `plugins/` directory itself replaced with a symlink to an external
/// directory: `plugins/<id>` would still be an ordinary-looking directory at
/// its own leaf, so `fs::remove_dir_all` on it would delete data this host
/// does not own. `fs::remove_dir_all` already treats a symlinked *child* as
/// an opaque entry to unlink rather than a target to recurse into, so only
/// the ancestry up to and including `path` itself needs checking here.
pub fn remove_owned_directory(trusted_base: &Path, path: &Path) -> std::io::Result<()> {
    validate_owned_descendant(trusted_base, path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_symlink() => Ok(()),
        Ok(_) => fs::remove_dir_all(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Removes every immediate child directory under `parent` that is not in
/// `referenced`, and every staging directory under `staging_root`.
///
/// `trusted_base` is the plugin host's own configuration root. `parent` and
/// `staging_root` must each be that root or a descendant of it, with no
/// symlink on the path between them — not just at their own leaf — since a
/// symlink planted at any point inside the owned tree (for example the
/// shared `plugins/` directory itself, one level above `parent` in the
/// common case) would otherwise make an ordinary-looking `parent` resolve
/// through it to unrelated external data. See [`validate_owned_descendant`].
/// This does not restrict anything *above* `trusted_base` — a plain
/// symlinked ancestor like macOS's `/tmp` -> `/private/tmp` is still fine.
pub fn reconcile(
    trusted_base: &Path,
    parent: &Path,
    referenced: &HashSet<PathBuf>,
    staging_root: &Path,
) -> std::io::Result<()> {
    reconcile_owned_children(trusted_base, parent, referenced)?;
    reconcile_owned_children(trusted_base, staging_root, &HashSet::new())?;
    Ok(())
}

fn reconcile_owned_children(
    trusted_base: &Path,
    root: &Path,
    referenced: &HashSet<PathBuf>,
) -> std::io::Result<()> {
    if validate_owned_descendant(trusted_base, root).is_err() {
        return Ok(());
    }
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    // Only an absent root is tolerated silently; every other `read_dir` or
    // per-entry error propagates. Reconciliation runs once at startup before
    // any concurrent install/uninstall — silently skipping a real error here
    // (permission denied, I/O failure) would let it masquerade as "nothing
    // to reconcile" instead of surfacing the failure it actually is.
    let read_dir = match fs::read_dir(root) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in read_dir {
        let entry = entry?;
        let path = entry.path();
        if !referenced.contains(&path) {
            remove_owned_directory(trusted_base, &path)?;
        }
    }
    Ok(())
}

/// Ensures `path` is safe for this host to treat as its own directory:
/// absent (created fresh) or already a plain directory, never a symlink or
/// other file type. A symlinked configuration root could otherwise redirect
/// writes (via a blind `create_dir_all`, which follows symlinks when
/// checking whether the destination already exists) or later deletions into
/// a directory this host does not own. `trusted_base` is validated first
/// (see [`validate_owned_descendant`]) so a symlink higher up the owned
/// tree than `path` itself is caught too.
fn ensure_owned_directory(trusted_base: &Path, path: &Path) -> std::io::Result<()> {
    validate_owned_descendant(trusted_base, path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_symlink() => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} is a symlink, not an owned directory", path.display()),
        )),
        Ok(metadata) if !metadata.is_dir() => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("{} exists and is not a directory", path.display()),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(path),
        Err(error) => Err(error),
    }
}

/// Validates that every path component strictly between `trusted_base`
/// (exclusive) and `target` (inclusive) is a plain directory, not a symlink.
///
/// Checking only `target`'s own leaf (as a naive symlink check would) misses
/// a symlink planted *above* the leaf but still inside the host's owned
/// storage tree: for example, if the shared `plugins/` directory itself were
/// replaced with a symlink to an external directory, `plugins/<id>` would
/// still resolve to an ordinary-looking directory at its own leaf — just one
/// that lives somewhere this host does not own. Anchoring the walk at
/// `trusted_base` (the plugin host's own configuration root) and checking
/// every component below it catches that case, while a symlink *above* or
/// *at* `trusted_base` itself (for example macOS's `/tmp` -> `/private/tmp`,
/// which every temp-directory-rooted path passes through) is untouched:
/// only components the host itself manages, below its own established base,
/// are required to be symlink-free.
///
/// The exact guarantee: this rejects a symlink that already exists on the
/// checked path *at the moment of this call* — the persistent-misconfiguration
/// or pre-positioned-attack case this host's own storage tree is exposed to.
/// It is a series of `symlink_metadata` calls by path, not a chain of open
/// file descriptors, so unlike `package.rs`'s `unix_fs` module (which reaches
/// every entry in the *untrusted source* tree through an already-open,
/// `O_NOFOLLOW` descriptor and so cannot be raced at all), this check and the
/// filesystem operation the caller performs afterward are not atomic: a
/// symlink swapped into the checked path *after* this function returns but
/// before that later operation runs is not caught. Closing that remaining
/// window would need the same descriptor-relative, no-follow-open approach
/// `unix_fs` uses, applied to every owned-storage write and delete, which is
/// a larger change than this fix.
fn validate_owned_descendant(trusted_base: &Path, target: &Path) -> std::io::Result<()> {
    let relative = target.strip_prefix(trusted_base).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{} is not inside the trusted base {}",
                target.display(),
                trusted_base.display()
            ),
        )
    })?;
    let mut probe = trusted_base.to_path_buf();
    for component in relative.components() {
        probe.push(component);
        match fs::symlink_metadata(&probe) {
            Ok(metadata) if metadata.is_symlink() => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{} is a symlink inside owned storage", probe.display()),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Nothing exists at or below this component yet, so there is
                // nothing further down the chain a symlink could have
                // redirected; the caller creates the rest fresh.
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn verify_target_executable(
    manifest: &Manifest,
    staged_path: &Path,
) -> Result<String, InstallReject> {
    let triple = env!("BUZZ_PLUGIN_TARGET_TRIPLE");
    let target = manifest
        .runtime
        .targets
        .get(triple)
        .ok_or(InstallReject::UnknownTarget)?;
    let executable_path = staged_path.join(&target.path);
    let metadata = fs::symlink_metadata(&executable_path)
        .map_err(|error| InstallReject::InvalidPackage(format!("stat executable: {error}")))?;
    if metadata.is_symlink() || !metadata.is_file() {
        return Err(InstallReject::InvalidPackage(
            "target executable must be a regular file".into(),
        ));
    }
    if metadata.len() != target.bytes {
        return Err(InstallReject::DigestMismatch);
    }
    let bytes = fs::read(&executable_path)
        .map_err(|error| InstallReject::InvalidPackage(format!("read executable: {error}")))?;
    let digest = hex::encode(Sha256::digest(&bytes));
    if digest != target.sha256 {
        return Err(InstallReject::DigestMismatch);
    }
    mark_executable(&executable_path)?;
    Ok(digest)
}

#[cfg(unix)]
fn mark_executable(path: &Path) -> Result<(), InstallReject> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(|error| InstallReject::InvalidPackage(format!("stat executable: {error}")))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| InstallReject::InvalidPackage(format!("chmod executable: {error}")))
}

#[cfg(not(unix))]
fn mark_executable(_path: &Path) -> Result<(), InstallReject> {
    Ok(())
}

/// Descriptor-relative, no-follow package tree copy.
///
/// Every directory and file is reached by `openat` from its parent's already
/// open descriptor rather than by re-resolving a path string, so a package
/// mutated on disk between two operations (rename, symlink swap, content
/// growth) cannot change what gets staged: the descriptor already refers to
/// whatever object existed at the moment it was opened. `O_NOFOLLOW` on every
/// open rejects a symlink introduced after `readdir` reported the entry
/// (`ELOOP`) instead of silently following it — including the package root
/// itself, opened the same way. Layout limits (file count, total bytes,
/// entry count, directory nesting) are enforced inline as the walk
/// proceeds, from the same descriptors, not from a separate preliminary
/// scan.
#[cfg(unix)]
mod unix_fs {
    use std::ffi::CStr;
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::path::Path;

    use nix::dir::Dir;
    use nix::fcntl::{self, OFlag};
    use nix::sys::stat::{self, mkdirat, mode_t, FileStat, Mode, SFlag};

    use crate::plugin_host::types::InstallReject;

    use super::{MAX_DIRECTORY_LEVELS, MAX_FILES, MAX_TOTAL_BYTES, MAX_TOTAL_ENTRIES};

    #[derive(Default)]
    struct WalkState {
        files: usize,
        entries: usize,
        total_bytes: u64,
    }

    pub(super) fn copy_validated_tree(
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<(), InstallReject> {
        let mut source_dir = open_dir_no_follow(source_root)?;
        std::fs::create_dir(dest_root).map_err(|error| {
            InstallReject::InvalidPackage(format!("create staging directory: {error}"))
        })?;
        let dest_dir_fd = fcntl::open(
            dest_root,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            InstallReject::InvalidPackage(format!("open staging directory: {error}"))
        })?;
        let mut state = WalkState::default();
        walk(&mut source_dir, &dest_dir_fd, 0, &mut state)
    }

    fn open_dir_no_follow(path: &Path) -> Result<Dir, InstallReject> {
        Dir::open(
            path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            InstallReject::InvalidPackage(format!(
                "open package directory {}: {error}",
                path.display()
            ))
        })
    }

    fn walk(
        source_dir: &mut Dir,
        dest_dir_fd: &OwnedFd,
        depth: usize,
        state: &mut WalkState,
    ) -> Result<(), InstallReject> {
        // `Dir::iter()` borrows `source_dir` mutably for the iterator's whole
        // lifetime, but the loop body also needs `source_dir` itself (to
        // `openat` each entry), so the entries are collected into an owned
        // list first and the borrow from `iter()` ends before the loop body
        // runs. `.take()` bounds how many raw dirents that collection ever
        // holds in memory — a handful over the real entry cap — regardless
        // of how many millions of entries an attacker-controlled directory
        // actually contains; the loop below still detects and rejects the
        // over-the-limit case from within that bounded prefix.
        let raw_entries: Vec<nix::dir::Entry> = source_dir
            .iter()
            .take(MAX_TOTAL_ENTRIES.saturating_add(4))
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| {
                InstallReject::InvalidPackage(format!("read directory entry: {error}"))
            })?;
        for entry in raw_entries {
            let name = entry.file_name();
            if name.to_bytes() == b"." || name.to_bytes() == b".." {
                continue;
            }

            state.entries += 1;
            if state.entries > MAX_TOTAL_ENTRIES {
                return Err(InstallReject::InvalidPackage(format!(
                    "package exceeds the {MAX_TOTAL_ENTRIES}-entry limit"
                )));
            }

            // Classify by the descriptor actually opened, not by a
            // preliminary `fstatat` on the name: a name that resolved to a
            // regular file (or anything else) at `fstatat` time could be
            // swapped for a different node type before a second, separate
            // open call reached it. `O_NONBLOCK` additionally guarantees
            // this open cannot hang even if the entry turns out to be a
            // FIFO by the time it's opened.
            let (fd, entry_stat) = open_and_stat(source_dir, name)?;
            let kind =
                SFlag::from_bits_truncate(entry_stat.st_mode as mode_t & SFlag::S_IFMT.bits());

            if kind == SFlag::S_IFDIR {
                // Nesting depth is a property of directories, not of the
                // files inside them: a file directly inside the 16th nested
                // directory is still fine, so this is only ever checked
                // before descending into another directory.
                let next_depth = depth + 1;
                if next_depth > MAX_DIRECTORY_LEVELS {
                    return Err(InstallReject::InvalidPackage(format!(
                        "package exceeds the {MAX_DIRECTORY_LEVELS}-directory-level limit"
                    )));
                }
                let mut child_source = Dir::from_fd(fd).map_err(|error| {
                    InstallReject::InvalidPackage(format!("open package subdirectory: {error}"))
                })?;
                let child_dest_fd = create_dest_dir(dest_dir_fd, name)?;
                walk(&mut child_source, &child_dest_fd, next_depth, state)?;
            } else if kind == SFlag::S_IFREG {
                state.files += 1;
                if state.files > MAX_FILES {
                    return Err(InstallReject::InvalidPackage(format!(
                        "package exceeds the {MAX_FILES}-file limit"
                    )));
                }
                let size = u64::try_from(entry_stat.st_size).unwrap_or(0);
                state.total_bytes = state.total_bytes.saturating_add(size);
                if state.total_bytes > MAX_TOTAL_BYTES {
                    return Err(InstallReject::InvalidPackage(format!(
                        "package exceeds the {MAX_TOTAL_BYTES}-byte limit"
                    )));
                }
                #[cfg(test)]
                super::test_hooks::after_size_observed(entry.ino());

                let mut source_file = File::from(fd);
                let mut dest_file = create_dest_file(dest_dir_fd, name)?;
                // Bound the actual bytes copied to the length observed at
                // `fstat` time: a file grown after that observation (mutation
                // during copy) cannot land more bytes in the staged copy than
                // were counted against `MAX_TOTAL_BYTES` above.
                let mut limited = (&mut source_file).take(size);
                std::io::copy(&mut limited, &mut dest_file).map_err(|error| {
                    InstallReject::InvalidPackage(format!("copy package file: {error}"))
                })?;
            } else {
                return Err(InstallReject::InvalidPackage(
                    "only regular files and directories are allowed".into(),
                ));
            }
        }
        Ok(())
    }

    /// Opens `name` under `parent` exactly once and reports its real type
    /// from the resulting descriptor's own `fstat`, so classification and
    /// use always agree on the same object — no separate stat-then-open
    /// pair for an attacker to race. `O_NONBLOCK` means this call cannot
    /// block even if `name` names a FIFO; `O_NOFOLLOW` rejects a symlink.
    fn open_and_stat(parent: &Dir, name: &CStr) -> Result<(OwnedFd, FileStat), InstallReject> {
        let fd = fcntl::openat(
            parent,
            name,
            OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| InstallReject::InvalidPackage(format!("open package entry: {error}")))?;
        let entry_stat = stat::fstat(&fd).map_err(|error| {
            InstallReject::InvalidPackage(format!("stat package entry: {error}"))
        })?;
        Ok((fd, entry_stat))
    }

    fn create_dest_dir(parent_fd: &OwnedFd, name: &CStr) -> Result<OwnedFd, InstallReject> {
        mkdirat(parent_fd, name, Mode::S_IRWXU).map_err(|error| {
            InstallReject::InvalidPackage(format!("create staging subdirectory: {error}"))
        })?;
        fcntl::openat(
            parent_fd,
            name,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            InstallReject::InvalidPackage(format!("open staging subdirectory: {error}"))
        })
    }

    fn create_dest_file(parent_fd: &OwnedFd, name: &CStr) -> Result<File, InstallReject> {
        let fd = fcntl::openat(
            parent_fd,
            name,
            OFlag::O_WRONLY | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::S_IRUSR | Mode::S_IWUSR,
        )
        .map_err(|error| InstallReject::InvalidPackage(format!("create staged file: {error}")))?;
        Ok(File::from(fd))
    }
}

/// Path-based fallback for non-unix targets, which have no `openat`/`O_NOFOLLOW`
/// via `std` without an extra dependency. Still a single pass (no separate
/// scan-then-copy phase) with the same inline limits and a symlink check
/// immediately before each entry is used, though without the descriptor-level
/// TOCTOU guarantee `unix_fs` provides.
#[cfg(not(unix))]
mod fallback_fs {
    use std::fs;
    use std::io::Read;
    use std::path::{Path, PathBuf};

    use crate::plugin_host::types::InstallReject;

    use super::{MAX_DIRECTORY_LEVELS, MAX_FILES, MAX_TOTAL_BYTES, MAX_TOTAL_ENTRIES};

    #[derive(Default)]
    struct WalkState {
        files: usize,
        entries: usize,
        total_bytes: u64,
    }

    pub(super) fn copy_validated_tree(
        source_root: &Path,
        dest_root: &Path,
    ) -> Result<(), InstallReject> {
        let root_metadata = fs::symlink_metadata(source_root).map_err(|error| {
            InstallReject::InvalidPackage(format!("open package root: {error}"))
        })?;
        if root_metadata.is_symlink() {
            return Err(InstallReject::InvalidPackage(
                "package root must not be a symlink".into(),
            ));
        }
        fs::create_dir(dest_root).map_err(|error| {
            InstallReject::InvalidPackage(format!("create staging directory: {error}"))
        })?;
        let mut state = WalkState::default();
        walk(source_root, dest_root, PathBuf::new(), 0, &mut state)
    }

    fn walk(
        source_root: &Path,
        dest_root: &Path,
        relative: PathBuf,
        depth: usize,
        state: &mut WalkState,
    ) -> Result<(), InstallReject> {
        let read_dir = fs::read_dir(source_root.join(&relative))
            .map_err(|error| InstallReject::InvalidPackage(format!("read directory: {error}")))?;
        for entry in read_dir {
            let entry = entry
                .map_err(|error| InstallReject::InvalidPackage(format!("read entry: {error}")))?;
            let entry_relative = relative.join(entry.file_name());

            state.entries += 1;
            if state.entries > MAX_TOTAL_ENTRIES {
                return Err(InstallReject::InvalidPackage(format!(
                    "package exceeds the {MAX_TOTAL_ENTRIES}-entry limit"
                )));
            }
            if entry_relative.components().count() > MAX_DIRECTORY_LEVELS {
                return Err(InstallReject::InvalidPackage(format!(
                    "package exceeds the {MAX_DIRECTORY_LEVELS}-directory-level limit: {}",
                    entry_relative.display()
                )));
            }

            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| InstallReject::InvalidPackage(format!("stat entry: {error}")))?;
            if metadata.is_symlink() {
                return Err(InstallReject::InvalidPackage(format!(
                    "symlinks are not allowed: {}",
                    entry_relative.display()
                )));
            }
            if metadata.is_dir() {
                fs::create_dir(dest_root.join(&entry_relative)).map_err(|error| {
                    InstallReject::InvalidPackage(format!("create staging subdirectory: {error}"))
                })?;
                walk(source_root, dest_root, entry_relative, depth + 1, state)?;
            } else if metadata.is_file() {
                state.files += 1;
                if state.files > MAX_FILES {
                    return Err(InstallReject::InvalidPackage(format!(
                        "package exceeds the {MAX_FILES}-file limit"
                    )));
                }
                state.total_bytes = state.total_bytes.saturating_add(metadata.len());
                if state.total_bytes > MAX_TOTAL_BYTES {
                    return Err(InstallReject::InvalidPackage(format!(
                        "package exceeds the {MAX_TOTAL_BYTES}-byte limit"
                    )));
                }
                let mut source_file = fs::File::open(entry.path()).map_err(|error| {
                    InstallReject::InvalidPackage(format!("open package file: {error}"))
                })?;
                let mut dest_file =
                    fs::File::create(dest_root.join(&entry_relative)).map_err(|error| {
                        InstallReject::InvalidPackage(format!("create staged file: {error}"))
                    })?;
                let mut limited = (&mut source_file).take(metadata.len());
                std::io::copy(&mut limited, &mut dest_file).map_err(|error| {
                    InstallReject::InvalidPackage(format!("copy package file: {error}"))
                })?;
            } else {
                return Err(InstallReject::InvalidPackage(format!(
                    "only regular files and directories are allowed: {}",
                    entry_relative.display()
                )));
            }
        }
        Ok(())
    }
}

/// Deterministic test-only seam for the mutation-during-copy regression: a
/// real filesystem race would need a sleep to reproduce reliably, so instead
/// a hook fires from inside the walk at the exact point right after a source
/// file's copy budget has been observed (`fstat`) but before the bounded
/// copy reads it. Scoped by inode number (not name or path) so it fires only
/// for the specific file a test cares about, even though `cargo test` runs
/// many `stage()` calls concurrently against this one process-wide hook.
#[cfg(test)]
mod test_hooks {
    use std::sync::Mutex;

    type Hook = Box<dyn Fn(u64) + Send + Sync>;

    static AFTER_SIZE_OBSERVED: Mutex<Option<Hook>> = Mutex::new(None);

    pub(super) fn set_after_size_observed(hook: impl Fn(u64) + Send + Sync + 'static) {
        *AFTER_SIZE_OBSERVED
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(Box::new(hook));
    }

    pub(super) fn clear() {
        *AFTER_SIZE_OBSERVED
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
    }

    pub(super) fn after_size_observed(ino: u64) {
        if let Some(hook) = AFTER_SIZE_OBSERVED
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
        {
            hook(ino);
        }
    }
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod tests;
