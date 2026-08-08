//! Vault path validation — the security core of the Documents feature.
//!
//! Ported from the Onyx editor (`onyx/src-tauri/src/lib.rs`), with two
//! deliberate hardenings noted on [`VaultState`] and [`ValidatedVaultPath`].

use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

/// A path that has passed [`validate_vault_path`].
///
/// Onyx enforced "use the returned `PathBuf`, never the string you passed in"
/// with a doc comment. Wrapping it in a newtype that the `fs::` helpers are the
/// only consumers of makes the same rule a compile error instead of a review
/// note — see the module docs on why that rule is load-bearing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedVaultPath(PathBuf);

impl ValidatedVaultPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}

/// The active vault root, held in Rust rather than passed from the renderer.
///
/// Onyx's commands took `(path, vault_path)` — letting a caller supply *both*
/// sides of the containment check, so `read_file("/etc/passwd", "/etc")` passed.
/// Five of its commands (`list_files`, `file_exists`, `get_file_modified_time`,
/// `get_file_stats`, `list_assets`) skipped validation entirely.
///
/// A content security policy does not help here: CSP constrains what the page
/// may load and execute, not what the app's own code may ask the backend to do.
/// Every containment decision has to be made on this side of the IPC boundary,
/// which is why the renderer never gets to name the root.
#[derive(Default)]
pub struct VaultState {
    inner: Mutex<Option<VaultRoots>>,
}

#[derive(Clone)]
struct VaultRoots {
    /// The spelling the user chose, which may itself be a symlink.
    literal: PathBuf,
    /// Its canonical spelling, when that differs.
    canonical: Option<PathBuf>,
}

impl VaultState {
    /// Records `root` as the active vault. Returns the stored literal path.
    pub fn set(&self, root: PathBuf) -> Result<PathBuf, String> {
        let canonical = root.canonicalize().ok().filter(|c| c != &root);
        let roots = VaultRoots {
            literal: root.clone(),
            canonical,
        };
        let mut guard = self.inner.lock().map_err(|e| e.to_string())?;
        *guard = Some(roots);
        Ok(root)
    }

    pub fn clear(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().map_err(|e| e.to_string())?;
        *guard = None;
        Ok(())
    }

    /// The active vault root, or `None` when no vault has been chosen.
    pub fn root(&self) -> Result<Option<PathBuf>, String> {
        let guard = self.inner.lock().map_err(|e| e.to_string())?;
        Ok(guard.as_ref().map(|roots| roots.literal.clone()))
    }

    /// The active vault root, erroring when none is set.
    pub fn require_root(&self) -> Result<PathBuf, String> {
        self.root()?
            .ok_or_else(|| "No vault folder is selected.".to_string())
    }

    /// Validates `path` against the active vault.
    ///
    /// This is the only way to obtain a [`ValidatedVaultPath`].
    pub fn validate(&self, path: &str) -> Result<ValidatedVaultPath, String> {
        let roots = {
            let guard = self.inner.lock().map_err(|e| e.to_string())?;
            guard
                .clone()
                .ok_or_else(|| "No vault folder is selected.".to_string())?
        };
        validate_vault_path(path, &roots.literal, roots.canonical.as_deref())
    }
}

/// Resolves `.` and `..` textually, without touching the filesystem.
fn lexically_normalize(path: &Path) -> Result<PathBuf, String> {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // `pop` returns false once we are back at the prefix/root.
                if !resolved.pop() {
                    return Err(format!(
                        "Invalid path: '{}' climbs above the filesystem root",
                        path.display()
                    ));
                }
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    Ok(resolved)
}

/// Validates that a path is within the allowed vault directory.
///
/// Containment is decided **lexically**, not by `canonicalize()`. Canonicalizing
/// first resolves symlinks, so a directory the user deliberately linked into
/// their vault (`Vault/Projects -> ~/Projects/onyx`) resolves to a path outside
/// the vault and every file under it gets rejected — while still being listed in
/// the tree, because the tree builder never canonicalizes. Linked folders are an
/// intentional grant by the person who created the link, so we follow them; `..`
/// escapes are still rejected, which is what the traversal guard is for.
///
/// **Callers must use the returned path and never the string they passed in.**
/// This is the whole of the rule's safety, not hygiene. `..` after a symlink hop
/// normalizes differently from how the OS would resolve it — e.g.
/// `<vault>/links/dir-outside/../../outside/notes/x.md` is allowed and names
/// `<vault>/outside/notes/x.md`, while the OS would have resolved it against the
/// link's target and reached somewhere else entirely. A caller that validates
/// `&p` and then touches `&p` is checking one path and operating on another.
/// [`ValidatedVaultPath`] is what stops a new call site from doing that.
fn validate_vault_path(
    path: &str,
    vault: &Path,
    canonical_vault: Option<&Path>,
) -> Result<ValidatedVaultPath, String> {
    let path = Path::new(path);

    if path.is_relative() {
        return Err(format!(
            "Invalid path: '{}' must be absolute",
            path.display()
        ));
    }

    let normalized_path = lexically_normalize(path)?;
    let normalized_vault = lexically_normalize(vault)?;

    // The vault itself may live behind a symlink, in which case callers can hold
    // either spelling of it. Accept both, and only the vault's own two forms —
    // this is a prefix test on the vault, never on the requested path.
    let mut allowed_roots = vec![normalized_vault];
    if let Some(canonical) = canonical_vault {
        let normalized_canonical = lexically_normalize(canonical)?;
        if !allowed_roots.contains(&normalized_canonical) {
            allowed_roots.push(normalized_canonical);
        }
    }

    if !allowed_roots
        .iter()
        .any(|root| normalized_path.starts_with(root))
    {
        return Err(format!(
            "Access denied: path '{}' is outside the vault directory",
            path.display()
        ));
    }

    Ok(ValidatedVaultPath(normalized_path))
}

/// Rejects moving a directory into its own subtree (`mv A A/B`).
///
/// `fs::rename` happens to return EINVAL for this on Linux, but the check is
/// cheap, portable, and produces a message a user can act on. Onyx's
/// `rename_file` has no equivalent guard.
pub fn reject_move_into_self(
    source: &ValidatedVaultPath,
    destination: &ValidatedVaultPath,
) -> Result<(), String> {
    if destination.as_path().starts_with(source.as_path()) {
        return Err("Cannot move a folder into itself.".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Builds a throwaway vault:
    ///   <tmp>/vault/Notes/plain.md
    ///   <tmp>/vault/Projects -> <tmp>/real/onyx   (the shape that was broken)
    ///   <tmp>/real/onyx/README.md
    fn fixture(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("buzz-vault-test-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("vault/Notes")).unwrap();
        fs::create_dir_all(root.join("real/onyx")).unwrap();
        fs::write(root.join("vault/Notes/plain.md"), "# plain").unwrap();
        fs::write(root.join("real/onyx/README.md"), "# linked").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("real/onyx"), root.join("vault/Projects")).unwrap();
        root
    }

    /// Validates against the fixture vault, mirroring how `VaultState` calls it.
    fn check(path: &str, root: &Path) -> Result<ValidatedVaultPath, String> {
        let vault = root.join("vault");
        let canonical = vault.canonicalize().ok().filter(|c| c != &vault);
        validate_vault_path(path, &vault, canonical.as_deref())
    }

    #[test]
    fn accepts_an_ordinary_file_in_the_vault() {
        let root = fixture("ordinary");
        let target = root.join("vault/Notes/plain.md");
        assert!(check(&target.to_string_lossy(), &root).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn accepts_a_file_inside_a_linked_folder() {
        // This is the regression: canonicalize() resolved the path to
        // <tmp>/real/onyx/README.md and the starts_with check rejected it.
        let root = fixture("linked");
        let target = root.join("vault/Projects/README.md");
        let resolved = check(&target.to_string_lossy(), &root)
            .expect("a file inside a linked folder must be readable");
        assert_eq!(
            resolved.as_path(),
            target,
            "the link path is preserved, not resolved"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn accepts_the_link_itself() {
        let root = fixture("linkdir");
        let target = root.join("vault/Projects");
        assert!(check(&target.to_string_lossy(), &root).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn accepts_a_file_that_does_not_exist_yet() {
        // New-note creation inside a linked folder.
        let root = fixture("newfile");
        let target = root.join("vault/Projects/brand-new.md");
        assert!(check(&target.to_string_lossy(), &root).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn still_rejects_dot_dot_traversal() {
        let root = fixture("traversal");
        let vault = root.join("vault");
        let target = format!("{}/Notes/../../real/onyx/README.md", vault.display());
        assert!(
            check(&target, &root).is_err(),
            "`..` out of the vault must stay denied"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn still_rejects_an_unrelated_absolute_path() {
        let root = fixture("outside");
        assert!(check("/etc/passwd", &root).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_a_relative_path() {
        let root = fixture("relative");
        assert!(check("Notes/plain.md", &root).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_a_sibling_that_merely_shares_a_name_prefix() {
        // "<tmp>/vault-evil" must not pass a naive starts_with on "<tmp>/vault".
        let root = fixture("prefix");
        fs::create_dir_all(root.join("vault-evil")).unwrap();
        fs::write(root.join("vault-evil/secret.md"), "x").unwrap();
        let target = root.join("vault-evil/secret.md");
        assert!(
            check(&target.to_string_lossy(), &root).is_err(),
            "Path::starts_with is component-wise, so this must be denied"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// Enough `..` after a symlink hop to leave the vault. Lexical normalization
    /// pops the link component rather than climbing out of the link's *target*,
    /// so the two orderings disagree — but both land outside the vault here, and
    /// the lexical one is denied. Deny is the safe direction.
    #[cfg(unix)]
    #[test]
    fn dot_dot_after_a_link_hop_out_of_the_vault_is_denied() {
        let root = fixture("linkhop-out");
        let vault = root.join("vault");
        fs::write(root.join("real/secret.md"), "secret").unwrap();
        let attack = format!("{}/Projects/../../real/secret.md", vault.display());
        assert!(
            check(&attack, &root).is_err(),
            "`..` climbing past the vault root must be denied even after a link hop"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// The other half: a single `..` after a link hop stays inside the vault
    /// lexically, while the OS would have resolved it against the link's target
    /// and landed elsewhere. That divergence is only safe because callers operate
    /// on the returned path, never the raw string — this test pins the returned
    /// path, which is what makes the rule sound.
    #[cfg(unix)]
    #[test]
    fn dot_dot_after_a_link_hop_pops_the_link_not_its_target() {
        let root = fixture("linkhop-pop");
        let vault = root.join("vault");
        let input = format!("{}/Projects/../Notes/plain.md", vault.display());

        let resolved = check(&input, &root).expect("lexically this lands back inside the vault");
        assert_eq!(
            resolved.as_path(),
            vault.join("Notes/plain.md"),
            "the returned path must pop the link component, not follow it"
        );
        assert!(
            resolved.as_path().exists(),
            "and it must name the real in-vault file"
        );
        // The OS ordering would have reached <tmp>/real/Notes/plain.md.
        assert!(!root.join("real/Notes/plain.md").exists());
        let _ = fs::remove_dir_all(&root);
    }

    /// The returned path is the security boundary: every command operates on it,
    /// so it must never point outside the vault even when the raw input would.
    #[test]
    fn returned_path_is_always_inside_the_vault() {
        let root = fixture("returned");
        let vault = root.join("vault");
        let mut inputs = vec![format!("{}/Notes/./plain.md", vault.display())];
        #[cfg(unix)]
        {
            inputs.push(format!("{}/Projects/README.md", vault.display()));
            inputs.push(format!("{}/Projects/../Notes/plain.md", vault.display()));
        }
        for input in inputs {
            let resolved = check(&input, &root).expect(&input);
            assert!(
                resolved.as_path().starts_with(&vault),
                "{} resolved outside the vault: {:?}",
                input,
                resolved
            );
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rejects_every_path_when_no_vault_is_set() {
        let state = VaultState::default();
        assert!(state.root().unwrap().is_none());
        assert!(state.require_root().is_err());
        assert!(state.validate("/tmp/anything.md").is_err());
    }

    #[test]
    fn validate_goes_through_the_active_vault_state() {
        let root = fixture("state");
        let state = VaultState::default();
        state.set(root.join("vault")).unwrap();

        let target = root.join("vault/Notes/plain.md");
        assert!(state.validate(&target.to_string_lossy()).is_ok());
        assert!(state.validate("/etc/passwd").is_err());

        state.clear().unwrap();
        assert!(
            state.validate(&target.to_string_lossy()).is_err(),
            "clearing the vault must revoke access"
        );
        let _ = fs::remove_dir_all(&root);
    }
}
