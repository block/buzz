use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Session-scoped utility aliases. Git is configured by the ACP harness.
///
/// Cleaned up on drop (TempDir) in the common case. If this process is killed
/// before `Drop` runs, a future buzz-dev-mcp startup's orphan sweep
/// (`crate::sweep`) removes the directory once this process's pid is confirmed
/// dead and no command spawned from it still holds the directory lease.
pub struct Shim {
    /// Declared before `_dir`, and it has to stay that way: fields drop in
    /// declaration order, and on Windows the lease file must be closed before
    /// `TempDir` can remove the directory holding it.
    _claim: Option<crate::sweep::DirClaim>,
    _dir: TempDir,
    pub path_env: String,
}

impl Shim {
    pub fn install() -> std::io::Result<Self> {
        let dir = tempfile::Builder::new().prefix("buzz-dev-mcp-").tempdir()?;
        set_owner_only(dir.path())?;
        // Ownership claim for the startup orphan sweep (#6025), best-effort,
        // see crate::sweep docs for why a failure here is non-fatal.
        let claim = crate::sweep::claim_dir(dir.path());

        let self_exe = std::env::current_exe()?;
        for name in ["rg", "tree", "buzz"] {
            symlink(&self_exe, &dir.path().join(name))?;
        }
        let original = std::env::var_os("PATH").unwrap_or_default();
        let mut entries = vec![PathBuf::from(dir.path())];
        entries.extend(std::env::split_paths(&original));
        let path_env = std::env::join_paths(entries)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
            .to_string_lossy()
            .into_owned();
        Ok(Self {
            _claim: claim,
            _dir: dir,
            path_env,
        })
    }

    /// Path of the shim directory. Used by the shell tool to surrender the
    /// ownership claim when a spawned command's lifetime stops being
    /// observable (see `sweep::surrender_claim`).
    pub fn dir(&self) -> &Path {
        self._dir.path()
    }
}

#[cfg(unix)]
fn set_owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_owner_only(_: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(not(unix))]
fn symlink(src: &Path, dst: &Path) -> std::io::Result<()> {
    // No symlinks without elevation on Windows; copy instead. The target needs
    // a .exe extension or PATH lookup (via PATHEXT) won't treat it as runnable.
    let dst = dst.with_extension("exe");
    std::fs::copy(src, dst).map(|_| ())
}

pub fn artifact_dir(session_root: &Path) -> PathBuf {
    let p = session_root.join("artifacts");
    let _ = std::fs::create_dir_all(&p);
    p
}
