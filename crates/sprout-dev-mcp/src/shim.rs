use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub struct Shim {
    _dir: TempDir,
    pub path_env: String,
}

impl Shim {
    pub fn install() -> std::io::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("sprout-dev-mcp-")
            .tempdir()?;
        set_owner_only(dir.path())?;

        let self_exe = std::env::current_exe()?;
        let rg_link = dir.path().join("rg");
        symlink(&self_exe, &rg_link)?;

        let tree_link = dir.path().join("tree");
        symlink(&self_exe, &tree_link)?;

        let sprout_link = dir.path().join("sprout");
        symlink(&self_exe, &sprout_link)?;

        let original = std::env::var_os("PATH").unwrap_or_default();
        let mut new_path = std::ffi::OsString::from(dir.path());
        if !original.is_empty() {
            new_path.push(":");
            new_path.push(&original);
        }
        let path_env = new_path.to_string_lossy().into_owned();

        Ok(Self {
            _dir: dir,
            path_env,
        })
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
    std::fs::copy(src, dst).map(|_| ())
}

pub fn artifact_dir(session_root: &Path) -> PathBuf {
    let p = session_root.join("artifacts");
    let _ = std::fs::create_dir_all(&p);
    p
}
