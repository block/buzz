//! Open an artifact referenced by a `buzz://file` deep link.
//!
//! The frontend parses those links in `shared/lib/fileLink.ts`, but that parse
//! is a convenience filter, not the security boundary — anything that can reach
//! the IPC layer bypasses it. The containment check therefore lives here, and
//! this module must be safe to call with fully attacker-controlled arguments.

use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::managed_agents::nest_dir;

/// Directory name of the nest entry that points at the active workspace's
/// repos directory. It is a symlink whenever the user has pointed the repos
/// directory outside the nest, which is exactly why `Repos` is a distinct root
/// rather than a path under `Nest`: a nest-rooted path traversing this entry
/// canonicalizes outside the nest and is (correctly) rejected.
const REPOS_ENTRY: &str = "REPOS";

/// The roots a `buzz://file` link may address. Mirrors `FileLinkRoot` in
/// `desktop/src/shared/lib/fileLink.ts`.
///
/// Deliberately closed: there is no variant for an arbitrary absolute path.
/// Adding one would make any chat message a filesystem opener.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileLinkRoot {
    /// The nest root — `~/.buzz`, or `~/.buzz-dev` on dev builds.
    Nest,
    /// The active workspace's repos directory.
    Repos,
}

/// Reject a link path that is empty, absolute, or able to escape its root, and
/// return it rebuilt from its plain components.
///
/// Uses [`Path::components`], so this catches `..` however it is spelled —
/// including the forms a naive `split('/')` check misses on Windows. Callers
/// still canonicalize afterwards; this is the cheap pre-filter that keeps a
/// hostile path from ever being joined.
///
/// `.` components are dropped rather than rejected. They are pure no-ops once
/// resolved, `components()` already elides the interior ones (`a/./b` yields
/// exactly `a` then `b`), and rejecting only the leading `./` would put this
/// out of step with `fileLink.ts`, which accepts both. A link the frontend
/// renders as a pill must not fail on click.
fn validate_relative_path(path: &str) -> Result<PathBuf, String> {
    if path.contains('\0') {
        return Err("file link path contains a null byte".into());
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err("file link path contains '..'".into()),
            Component::RootDir | Component::Prefix(_) => {
                return Err("file link path must be relative".into());
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err("file link path is empty".into());
    }
    Ok(normalized)
}

/// Resolve `root` to an existing directory on disk.
fn resolve_root(root: FileLinkRoot) -> Result<PathBuf, String> {
    let nest = nest_dir().ok_or("cannot resolve home directory for nest")?;
    let dir = match root {
        FileLinkRoot::Nest => nest,
        FileLinkRoot::Repos => nest.join(REPOS_ENTRY),
    };
    dir.canonicalize()
        .map_err(|e| format!("cannot resolve {} root: {e}", root_label(root)))
}

fn root_label(root: FileLinkRoot) -> &'static str {
    match root {
        FileLinkRoot::Nest => "nest",
        FileLinkRoot::Repos => "repos",
    }
}

/// Resolve a link path against its root, or explain why it is not openable.
///
/// The returned path is canonical, exists, and is inside the canonicalized
/// root. Canonicalizing *both* sides is what makes a planted symlink useless:
/// agents can write anywhere in the nest, so a symlink pointing at a credential
/// directory elsewhere in `$HOME` is a realistic way to turn a chat message
/// into an exfiltration primitive. Resolving before comparing closes it.
fn resolve_link_target(root: FileLinkRoot, path: &str) -> Result<PathBuf, String> {
    let relative = validate_relative_path(path)?;
    let root_dir = resolve_root(root)?;
    let target = root_dir
        .join(&relative)
        .canonicalize()
        .map_err(|_| format!("{path} does not exist in the {} root", root_label(root)))?;

    if !target.starts_with(&root_dir) {
        return Err(format!(
            "{path} resolves outside the {} root",
            root_label(root)
        ));
    }
    Ok(target)
}

/// Open — or reveal in the file manager — an artifact addressed by a
/// `buzz://file` link.
///
/// Returns `Err` with a human-readable reason the caller surfaces as a toast.
/// A missing file is an ordinary outcome, not a bug: artifacts get regenerated
/// and deleted, and a dead link must say so rather than silently do nothing.
#[tauri::command]
pub async fn open_workspace_file(
    app: AppHandle,
    path: String,
    root: FileLinkRoot,
    reveal: bool,
) -> Result<(), String> {
    let target = tokio::task::spawn_blocking(move || resolve_link_target(root, &path))
        .await
        .map_err(|e| format!("spawn_blocking failed: {e}"))??;

    let opener = app.opener();
    if reveal {
        opener
            .reveal_item_in_dir(&target)
            .map_err(|e| format!("failed to reveal file: {e}"))
    } else {
        opener
            .open_path(target.to_string_lossy(), None::<&str>)
            .map_err(|e| format!("failed to open file: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_plain_nested_path() {
        assert_eq!(
            validate_relative_path("docs/guides/NOTES.md").unwrap(),
            PathBuf::from("docs/guides/NOTES.md")
        );
    }

    #[test]
    fn rejects_absolute_and_traversing_paths() {
        for bad in ["", "/etc/passwd", "../outside", "a/../../b", ".", "./"] {
            assert!(
                validate_relative_path(bad).is_err(),
                "expected rejection for {bad:?}"
            );
        }
    }

    /// `.` components are no-ops, so they are dropped rather than rejected —
    /// and `fileLink.ts` accepts them too. Pinned because the two layers
    /// disagreeing would mean a pill that renders but fails on click.
    #[test]
    fn drops_no_op_current_dir_components() {
        assert_eq!(
            validate_relative_path("./docs/./P.md").unwrap(),
            PathBuf::from("docs/P.md")
        );
    }

    #[test]
    fn rejects_null_bytes() {
        assert!(validate_relative_path("a/\0/b").is_err());
    }

    #[test]
    fn resolves_a_file_inside_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("docs")).unwrap();
        std::fs::write(root.join("docs/NOTES.md"), b"hi").unwrap();

        let relative = validate_relative_path("docs/NOTES.md").unwrap();
        let target = root.join(&relative).canonicalize().unwrap();
        assert!(target.starts_with(&root));
    }

    /// The case that motivates canonicalizing both sides: a symlink planted
    /// inside the root pointing at a file outside it. A textual `..` check
    /// passes this path; the prefix check on canonicalized paths does not.
    #[cfg(unix)]
    #[test]
    fn rejects_a_symlink_that_escapes_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), b"s3cret").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();

        let root = root.canonicalize().unwrap();
        let relative = validate_relative_path("escape/secret").unwrap();
        let target = root.join(&relative).canonicalize().unwrap();

        assert!(
            !target.starts_with(&root),
            "symlink escape should not be contained by the root"
        );
    }

    #[test]
    fn root_deserializes_from_the_frontend_spelling() {
        assert_eq!(
            serde_json::from_str::<FileLinkRoot>("\"nest\"").unwrap(),
            FileLinkRoot::Nest
        );
        assert_eq!(
            serde_json::from_str::<FileLinkRoot>("\"repos\"").unwrap(),
            FileLinkRoot::Repos
        );
        assert!(serde_json::from_str::<FileLinkRoot>("\"home\"").is_err());
    }
}
