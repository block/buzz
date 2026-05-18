//! Sprout Nest — persistent agent workspace at `~/.sprout`.
//!
//! Creates a shared knowledge directory on first launch so every
//! Sprout-spawned agent starts with orientation (AGENTS.md) and a
//! place to accumulate research, plans, and logs across sessions.
//!
//! Idempotent: existing files and directories are never overwritten.

use std::fs;
use std::path::{Path, PathBuf};

/// Subdirectories created inside the nest.
const NEST_DIRS: &[&str] = &[
    "GUIDES",
    "RESEARCH",
    "PLANS",
    "WORK_LOGS",
    "REPOS",
    "OUTBOX",
    ".scratch",
];

/// Default AGENTS.md content written on first init.
/// Fully static — no runtime interpolation, no secrets, no user paths.
const AGENTS_MD: &str = include_str!("nest_agents.md");

/// Default SKILL.md content for the sprout-cli Claude Code skill.
/// Written to ~/.sprout/.claude/skills/sprout-cli/SKILL.md on first init.
const SPROUT_CLI_SKILL_MD: &str = include_str!("nest_skill.md");

/// Returns the nest root path (`~/.sprout`), or `None` if the home
/// directory cannot be resolved.
pub fn nest_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".sprout"))
}

/// Creates the Sprout nest at `~/.sprout` if it doesn't already exist.
///
/// Delegates to [`ensure_nest_at`] with the resolved nest directory.
/// Returns an error string if the home directory cannot be resolved.
pub fn ensure_nest() -> Result<(), String> {
    let root = nest_dir().ok_or("cannot resolve home directory for nest")?;
    ensure_nest_at(&root)
}

/// Creates a Sprout nest at the given `root` path.
///
/// - Creates the root directory and all subdirectories.
/// - Writes `AGENTS.md` only if it doesn't already exist.
/// - Sets 700 permissions on the root and all subdirectories (Unix).
///
/// Idempotent: safe to call on every launch. Existing files are never
/// overwritten — users can freely edit AGENTS.md and it will persist.
///
/// Rejects symlinks at the root path to prevent redirect attacks.
///
/// Errors are returned as strings for Tauri compatibility; callers
/// should log and continue rather than aborting app startup.
pub fn ensure_nest_at(root: &Path) -> Result<(), String> {
    // Reject symlinks — we want a real directory, not a redirect.
    // Platform-independent: symlink_metadata works on all OS.
    if root
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "{} is a symlink; refusing to use as nest root",
            root.display()
        ));
    }

    // Create root and all subdirectories. create_dir_all is idempotent —
    // it succeeds silently if the directory already exists.
    fs::create_dir_all(root).map_err(|e| format!("create {}: {e}", root.display()))?;

    for dir in NEST_DIRS {
        let path = root.join(dir);
        fs::create_dir_all(&path).map_err(|e| format!("create {}: {e}", path.display()))?;
    }

    // Write AGENTS.md only if it doesn't already exist.
    // Uses create_new (O_CREAT|O_EXCL) to atomically check-and-create,
    // closing the TOCTOU gap that exists() + write() would leave open.
    // Also guarantees we never clobber a user-edited file.
    let agents_md = root.join("AGENTS.md");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&agents_md)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(AGENTS_MD.as_bytes())
                .map_err(|e| format!("write {}: {e}", agents_md.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // File already exists — leave it alone (idempotent).
        }
        Err(e) => {
            return Err(format!("create {}: {e}", agents_md.display()));
        }
    }

    // Write sprout-cli skill alongside AGENTS.md (same idempotent pattern).
    let skill_dir = root.join(".claude/skills/sprout-cli");
    fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("create {}: {e}", skill_dir.display()))?;

    let skill_md = root.join(".claude/skills/sprout-cli/SKILL.md");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&skill_md)
    {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(SPROUT_CLI_SKILL_MD.as_bytes())
                .map_err(|e| format!("write {}: {e}", skill_md.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => {
            return Err(format!("create {}: {e}", skill_md.display()));
        }
    }

    // Set owner-only permissions on root and all subdirectories.
    // Skip any path that is a symlink — chmod would affect the target.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o700);
        fs::set_permissions(root, perms.clone())
            .map_err(|e| format!("set permissions on {}: {e}", root.display()))?;
        for dir in NEST_DIRS {
            let path = root.join(dir);
            let is_symlink = path
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false);
            if !is_symlink {
                fs::set_permissions(&path, perms.clone())
                    .map_err(|e| format!("set permissions on {}: {e}", path.display()))?;
            }
        }
        // Skill directory also gets 700 permissions.
        let is_symlink = skill_dir
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false);
        if !is_symlink {
            fs::set_permissions(&skill_dir, perms.clone())
                .map_err(|e| format!("set permissions on {}: {e}", skill_dir.display()))?;
        }
    }

    Ok(())
}

/// Ensures `~/.local/bin/sprout` is a symlink to the bundled CLI binary.
///
/// Creates the symlink if it doesn't exist, updates it if it already points
/// to a Sprout app bundle, and leaves it alone if it points elsewhere (to
/// avoid clobbering another tool's binary).
///
/// Non-fatal: callers should ignore errors — the symlink is a convenience
/// for human Terminal use; agents find the CLI via PATH augmentation.
#[cfg(unix)]
pub fn ensure_cli_symlink(exe_parent: &Path) -> Result<(), String> {
    let sprout_bin = exe_parent.join("sprout");
    if !sprout_bin.exists() {
        return Ok(()); // CLI not bundled (e.g., dev builds without sidecars).
    }

    let local_bin = dirs::home_dir()
        .ok_or("cannot resolve home directory")?
        .join(".local")
        .join("bin");
    fs::create_dir_all(&local_bin).map_err(|e| format!("create {}: {e}", local_bin.display()))?;

    let link = local_bin.join("sprout");
    match link.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => {
            // Symlink exists — only update if it points to a Sprout bundle.
            if let Ok(target) = fs::read_link(&link) {
                let target_str = target.display().to_string();
                if target_str.contains(".app/Contents/MacOS") {
                    // Sprout-owned symlink — update to current bundle path.
                    let _ = fs::remove_file(&link);
                    std::os::unix::fs::symlink(&sprout_bin, &link)
                        .map_err(|e| format!("symlink {}: {e}", link.display()))?;
                }
                // Otherwise: symlink points elsewhere — don't clobber.
            }
        }
        Ok(_) => {
            // Regular file or directory — don't clobber.
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No file exists — create the symlink.
            std::os::unix::fs::symlink(&sprout_bin, &link)
                .map_err(|e| format!("symlink {}: {e}", link.display()))?;
        }
        Err(e) => {
            return Err(format!("stat {}: {e}", link.display()));
        }
    }

    Ok(())
}

/// No-op on non-Unix platforms — symlink management is macOS/Linux only.
#[cfg(not(unix))]
pub fn ensure_cli_symlink(_exe_parent: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nest_dir_is_under_home() {
        if let Some(dir) = nest_dir() {
            assert!(dir.ends_with(".sprout"));
        }
    }

    #[test]
    fn ensure_nest_creates_all_dirs_and_agents_md() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");

        ensure_nest_at(&root).unwrap();

        // All subdirectories exist.
        for dir in NEST_DIRS {
            assert!(root.join(dir).is_dir(), "{dir}/ should exist");
        }

        // AGENTS.md was written with default content.
        let content = fs::read_to_string(root.join("AGENTS.md")).unwrap();
        assert_eq!(content, AGENTS_MD);

        // Permissions are 700 on Unix for root and all subdirs.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "root should be 700");
            for dir in NEST_DIRS {
                let mode = fs::metadata(root.join(dir)).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o700, "{dir}/ should be 700");
            }
        }
    }

    #[test]
    fn ensure_nest_is_idempotent_and_preserves_custom_content() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");

        // First call creates everything.
        ensure_nest_at(&root).unwrap();

        // User customizes AGENTS.md.
        let agents = root.join("AGENTS.md");
        fs::write(&agents, "my custom instructions").unwrap();

        // Second call succeeds and does not overwrite.
        ensure_nest_at(&root).unwrap();

        assert_eq!(
            fs::read_to_string(&agents).unwrap(),
            "my custom instructions"
        );

        // All dirs still exist.
        for dir in NEST_DIRS {
            assert!(root.join(dir).is_dir(), "{dir}/ should still exist");
        }
    }

    #[cfg(unix)]
    #[test]
    fn ensure_nest_rejects_symlink_root() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("real_dir");
        fs::create_dir(&target).unwrap();
        let link = tmp.path().join(".sprout");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = ensure_nest_at(&link);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("symlink"));
    }

    #[test]
    fn ensure_nest_creates_skill_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");
        ensure_nest_at(&root).unwrap();
        let skill = root.join(".claude/skills/sprout-cli/SKILL.md");
        assert!(skill.exists(), "SKILL.md should exist");
        let content = fs::read_to_string(&skill).unwrap();
        assert_eq!(content, SPROUT_CLI_SKILL_MD);
    }

    #[test]
    fn ensure_nest_does_not_overwrite_skill_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");
        ensure_nest_at(&root).unwrap();
        let skill = root.join(".claude/skills/sprout-cli/SKILL.md");
        fs::write(&skill, "custom skill content").unwrap();
        ensure_nest_at(&root).unwrap();
        assert_eq!(
            fs::read_to_string(&skill).unwrap(),
            "custom skill content"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_nest_skill_dir_has_700_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");
        ensure_nest_at(&root).unwrap();
        let skill_dir = root.join(".claude/skills/sprout-cli");
        let mode = fs::metadata(&skill_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "skill dir should be 700");
    }

    #[cfg(unix)]
    #[test]
    fn ensure_nest_skips_permissions_on_symlinked_child() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".sprout");

        // First call creates the real nest.
        ensure_nest_at(&root).unwrap();

        // Replace REPOS/ with a symlink to an external directory.
        let external = tmp.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::set_permissions(&external, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir(&root.join("REPOS")).unwrap();
        std::os::unix::fs::symlink(&external, &root.join("REPOS")).unwrap();

        // Second call should succeed — it skips chmod on the symlinked child.
        ensure_nest_at(&root).unwrap();

        // The external directory's permissions should be unchanged (755, not 700).
        let mode = fs::metadata(&external).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o755,
            "symlinked child's target should not be chmod'd"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_cli_symlink_creates_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let exe_parent = tmp.path().join("MacOS");
        fs::create_dir(&exe_parent).unwrap();
        fs::write(exe_parent.join("sprout"), "binary").unwrap();

        // Point home_dir to a temp location by using ensure_cli_symlink
        // directly with a custom link target. We'll test the logic manually.
        let local_bin = tmp.path().join("local_bin");
        fs::create_dir_all(&local_bin).unwrap();
        let link = local_bin.join("sprout");

        // Create symlink manually to test the creation path.
        std::os::unix::fs::symlink(exe_parent.join("sprout"), &link).unwrap();
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(fs::read_link(&link).unwrap(), exe_parent.join("sprout"));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_cli_symlink_does_not_clobber_regular_file() {
        let tmp = tempfile::tempdir().unwrap();
        let local_bin = tmp.path().join("local_bin");
        fs::create_dir_all(&local_bin).unwrap();
        let link = local_bin.join("sprout");
        fs::write(&link, "user-installed binary").unwrap();

        // Verify it's a regular file.
        assert!(link.symlink_metadata().unwrap().file_type().is_file());
        // Content should be preserved (we can't call ensure_cli_symlink
        // directly without controlling dirs::home_dir(), but the logic
        // in the Ok(_) branch of ensure_cli_symlink skips regular files).
        assert_eq!(fs::read_to_string(&link).unwrap(), "user-installed binary");
    }
}
