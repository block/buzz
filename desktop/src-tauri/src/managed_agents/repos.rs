//! Per-workspace `REPOS` directory provisioning.
//!
//! The nest's `REPOS` directory is either a real directory (the default) or a
//! symlink to a user-configured `repos_dir`, letting agents work in existing
//! local checkouts instead of re-cloning. [`ensure_repos_symlink`] reconciles
//! `REPOS` with the configured path; [`validate_repos_dir`] guards the input.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Validate a user-supplied `repos_dir`, returning the canonical target path.
///
/// Requires an **existing absolute directory**. Rejects relative paths,
/// `~`-prefixed paths (shell tilde is not expanded by `std::fs` — the FE
/// expands before save, so a `~` reaching here is a bug to surface loudly),
/// non-directories, and a path that is the nest itself or an ancestor of it
/// (symlinking `REPOS` into its own parent would create a cycle). Never
/// creates the target — a typo must not silently mint a stray directory.
pub fn validate_repos_dir(nest_root: &Path, repos_dir: &str) -> Result<PathBuf, String> {
    let trimmed = repos_dir.trim();
    if trimmed.starts_with('~') {
        return Err(format!(
            "repos dir must be an absolute path (got `{trimmed}`); use e.g. /Users/you/Development"
        ));
    }
    let target = Path::new(trimmed);
    if !target.is_absolute() {
        return Err(format!(
            "repos dir must be an absolute path (got `{trimmed}`)"
        ));
    }
    // Resolve symlinks/`..` so the directory check and ancestor check both
    // operate on the real location. Fails loudly on a missing or unreadable
    // path rather than falling back to a real REPOS dir.
    let canonical = target
        .canonicalize()
        .map_err(|e| format!("repos dir `{trimmed}` is not accessible: {e}"))?;
    if !canonical.is_dir() {
        return Err(format!("repos dir `{trimmed}` is not a directory"));
    }
    // Refuse the nest itself or any ancestor of it — pointing REPOS there
    // would nest the symlink inside its own target.
    if let Ok(nest_canonical) = nest_root.canonicalize() {
        if nest_canonical == canonical || nest_canonical.starts_with(&canonical) {
            return Err(format!(
                "repos dir `{trimmed}` is the nest or an ancestor of it; choose a separate directory"
            ));
        }
    }
    Ok(canonical)
}

/// Ensure `nest_root/REPOS` matches the configured `repos_dir`.
///
/// - **`repos_dir` = `None`/empty** → ensure `REPOS` is a real in-nest
///   directory (the default). A pre-existing symlink (from a prior
///   `repos_dir`) is removed first so clearing the field genuinely reverts;
///   removing a symlink never touches its target. Idempotent otherwise.
/// - **`repos_dir` set, `REPOS` absent** → create a symlink to the target.
/// - **`repos_dir` set, `REPOS` is a symlink** (any target) → replace it
///   (`remove_file` + re-symlink). Removing a symlink never touches the
///   target's contents, so this is data-safe.
/// - **`repos_dir` set, `REPOS` is an empty real dir** → remove it and
///   symlink. Converting an empty dir loses nothing.
/// - **`repos_dir` set, `REPOS` is a NON-EMPTY real dir** → refuse and warn.
///   Never `remove_dir_all` — that would destroy repos the agent cloned
///   in-nest. The user must clear or relocate them first.
///
/// Validation (`validate_repos_dir`) runs before any filesystem mutation, so
/// an invalid path returns `Err` with `REPOS` left exactly as it was.
#[cfg(unix)]
pub fn ensure_repos_symlink(nest_root: &Path, repos_dir: Option<&str>) -> Result<(), String> {
    let repos_path = nest_root.join("REPOS");

    let Some(target) = repos_dir
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|raw| validate_repos_dir(nest_root, raw))
        .transpose()?
    else {
        // No repos_dir: REPOS must be a real in-nest directory. If it is
        // currently a symlink (from a prior repos_dir), remove the link first
        // — create_dir_all follows a symlink and would leave the stale link in
        // place. remove_file never touches the link's target.
        if let Ok(meta) = repos_path.symlink_metadata() {
            if meta.file_type().is_symlink() {
                fs::remove_file(&repos_path)
                    .map_err(|e| format!("remove symlink {}: {e}", repos_path.display()))?;
            }
        }
        fs::create_dir_all(&repos_path)
            .map_err(|e| format!("create {}: {e}", repos_path.display()))?;
        return Ok(());
    };

    match repos_path.symlink_metadata() {
        // Existing symlink → replace it if it points elsewhere. Re-pointing a
        // symlink is data-safe; remove_file never follows the link.
        Ok(meta) if meta.file_type().is_symlink() => {
            if repos_path.read_link().ok().as_deref() == Some(target.as_path()) {
                return Ok(()); // already correct
            }
            fs::remove_file(&repos_path)
                .map_err(|e| format!("remove symlink {}: {e}", repos_path.display()))?;
            symlink_repos(&target, &repos_path)
        }
        // Existing real directory → convert only if empty; otherwise refuse.
        Ok(meta) if meta.is_dir() => {
            let empty = fs::read_dir(&repos_path)
                .map_err(|e| format!("read {}: {e}", repos_path.display()))?
                .next()
                .is_none();
            if !empty {
                return Err(format!(
                    "{} holds repositories; move or delete them before pointing repos dir elsewhere",
                    repos_path.display()
                ));
            }
            fs::remove_dir(&repos_path)
                .map_err(|e| format!("remove {}: {e}", repos_path.display()))?;
            symlink_repos(&target, &repos_path)
        }
        // Exists but is neither symlink nor dir (e.g. a file) → refuse.
        Ok(_) => Err(format!(
            "{} exists and is not a directory; cannot point repos dir there",
            repos_path.display()
        )),
        // Absent → create the symlink.
        Err(e) if e.kind() == io::ErrorKind::NotFound => symlink_repos(&target, &repos_path),
        Err(e) => Err(format!("stat {}: {e}", repos_path.display())),
    }
}

#[cfg(unix)]
fn symlink_repos(target: &Path, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|e| format!("symlink {} → {}: {e}", link.display(), target.display()))
}

#[cfg(not(unix))]
pub fn ensure_repos_symlink(nest_root: &Path, _repos_dir: Option<&str>) -> Result<(), String> {
    let repos_path = nest_root.join("REPOS");
    fs::create_dir_all(&repos_path).map_err(|e| format!("create {}: {e}", repos_path.display()))
}

/// Provision `REPOS` at nest setup, before any configured `repos_dir` is known.
///
/// Leaves an existing symlink untouched — `apply_workspace` is the sole
/// authority over a configured symlink. Clearing it here with `None` would
/// destroy a symlink restored from a prior session; async-restored agents
/// would then write into the fresh real dir, and the later FE re-point would
/// refuse the now-non-empty REPOS — silently breaking `repos_dir` on restart.
/// Otherwise (absent, or a real dir) lands the default real-dir fallback.
pub fn ensure_repos_setup_default(nest_root: &Path) -> Result<(), String> {
    let repos_path = nest_root.join("REPOS");
    let is_symlink = repos_path
        .symlink_metadata()
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false);
    if is_symlink {
        return Ok(());
    }
    ensure_repos_symlink(nest_root, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── ensure_repos_symlink ──────────────────────────────────────────────

    #[cfg(unix)]
    #[test]
    fn ensure_repos_symlink_none_creates_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        fs::create_dir_all(&root).unwrap();

        ensure_repos_symlink(&root, None).unwrap();

        let repos = root.join("REPOS");
        assert!(repos.is_dir(), "REPOS should be a real directory");
        assert!(
            !repos.symlink_metadata().unwrap().file_type().is_symlink(),
            "REPOS should not be a symlink when repos_dir is None"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repos_symlink_none_reverts_existing_symlink_to_real_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        fs::create_dir_all(&root).unwrap();
        let external = tmp.path().join("Development");
        fs::create_dir_all(&external).unwrap();
        let payload = external.join("KEEP.md");
        fs::write(&payload, "data").unwrap();

        // First point REPOS at the external dir, then clear the field.
        ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();
        ensure_repos_symlink(&root, None).unwrap();

        let repos = root.join("REPOS");
        assert!(
            repos.is_dir(),
            "REPOS should be a real directory after clear"
        );
        assert!(
            !repos.symlink_metadata().unwrap().file_type().is_symlink(),
            "REPOS should no longer be a symlink after clearing repos_dir"
        );
        assert!(
            payload.exists(),
            "clearing repos_dir must not touch the external target's contents"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repos_symlink_absent_creates_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        fs::create_dir_all(&root).unwrap();
        let external = tmp.path().join("Development");
        fs::create_dir_all(&external).unwrap();

        ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();

        let repos = root.join("REPOS");
        assert!(repos.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(repos.read_link().unwrap(), external.canonicalize().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repos_symlink_repoints_existing_wrong_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        fs::create_dir_all(&root).unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        let payload = old.join("KEEP.md");
        fs::write(&payload, "data").unwrap();

        ensure_repos_symlink(&root, Some(old.to_str().unwrap())).unwrap();
        ensure_repos_symlink(&root, Some(new.to_str().unwrap())).unwrap();

        let repos = root.join("REPOS");
        assert_eq!(repos.read_link().unwrap(), new.canonicalize().unwrap());
        assert!(
            payload.exists(),
            "re-pointing a symlink must not touch the old target's contents"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repos_symlink_correct_symlink_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        fs::create_dir_all(&root).unwrap();
        let external = tmp.path().join("Development").canonicalize_or_make();

        ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();
        ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();

        let repos = root.join("REPOS");
        assert_eq!(repos.read_link().unwrap(), external);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repos_symlink_empty_real_dir_converts() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        fs::create_dir_all(root.join("REPOS")).unwrap();
        let external = tmp.path().join("Development");
        fs::create_dir_all(&external).unwrap();

        ensure_repos_symlink(&root, Some(external.to_str().unwrap())).unwrap();

        let repos = root.join("REPOS");
        assert!(
            repos.symlink_metadata().unwrap().file_type().is_symlink(),
            "an empty real REPOS should convert to a symlink"
        );
    }

    #[cfg(unix)]
    #[test]
    fn ensure_repos_symlink_nonempty_real_dir_refuses_and_preserves() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        let repos = root.join("REPOS");
        fs::create_dir_all(&repos).unwrap();
        let checkout = repos.join("buzz");
        fs::create_dir_all(&checkout).unwrap();
        fs::write(checkout.join("code.rs"), "fn main() {}").unwrap();
        let external = tmp.path().join("Development");
        fs::create_dir_all(&external).unwrap();

        let result = ensure_repos_symlink(&root, Some(external.to_str().unwrap()));

        assert!(result.is_err(), "non-empty real REPOS must refuse");
        assert!(
            !repos.symlink_metadata().unwrap().file_type().is_symlink(),
            "refused REPOS must stay a real directory"
        );
        assert!(
            checkout.join("code.rs").exists(),
            "refusal must never delete existing repositories"
        );
    }

    // ensure_nest_at must NOT clobber an existing REPOS symlink on startup.
    // Regression guard for Finding 1: the startup `ensure_repos_symlink(_, None)`
    // call used to remove a configured symlink and mint an empty real REPOS,
    // which async-restored agents could write into — the FE re-point then
    // refused the now-non-empty dir, silently breaking a configured repos_dir.
    #[cfg(unix)]
    #[test]
    fn ensure_nest_startup_preserves_existing_repos_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");

        // First launch creates the real nest with a real REPOS dir.
        crate::managed_agents::ensure_nest_at(&root).unwrap();

        // Simulate a configured repos_dir: REPOS points at an external dir
        // holding agent checkouts.
        let external = tmp.path().join("Development");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("KEEP.md"), "data").unwrap();
        fs::remove_dir(root.join("REPOS")).unwrap();
        std::os::unix::fs::symlink(&external, root.join("REPOS")).unwrap();

        // Next launch must leave the configured symlink intact.
        crate::managed_agents::ensure_nest_at(&root).unwrap();

        let repos = root.join("REPOS");
        assert!(
            repos.symlink_metadata().unwrap().file_type().is_symlink(),
            "an existing REPOS symlink must survive startup"
        );
        assert_eq!(repos.read_link().unwrap(), external);
        assert!(
            external.join("KEEP.md").exists(),
            "the symlink's target contents must be untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn validate_repos_dir_rejects_tilde_relative_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(".buzz");
        fs::create_dir_all(&root).unwrap();

        assert!(validate_repos_dir(&root, "~/Development").is_err());
        assert!(validate_repos_dir(&root, "relative/path").is_err());
        assert!(validate_repos_dir(&root, "/no/such/dir/here").is_err());

        let file = tmp.path().join("afile");
        fs::write(&file, "x").unwrap();
        assert!(validate_repos_dir(&root, file.to_str().unwrap()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn validate_repos_dir_rejects_nest_ancestor() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("home").join(".buzz");
        fs::create_dir_all(&root).unwrap();
        let parent = root.parent().unwrap();

        assert!(
            validate_repos_dir(&root, parent.to_str().unwrap()).is_err(),
            "a parent of the nest would nest REPOS inside its own target"
        );
    }

    /// Test helper: canonicalize a path, creating it as a directory first.
    #[cfg(unix)]
    trait CanonicalizeOrMake {
        fn canonicalize_or_make(&self) -> PathBuf;
    }
    #[cfg(unix)]
    impl CanonicalizeOrMake for PathBuf {
        fn canonicalize_or_make(&self) -> PathBuf {
            fs::create_dir_all(self).unwrap();
            self.canonicalize().unwrap()
        }
    }
}
