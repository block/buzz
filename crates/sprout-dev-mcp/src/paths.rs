//! Path resolution shared across dev-mcp tools.
//!
//! `resolve_within` canonicalises a user-supplied path against a workspace
//! root and rejects any result that escapes the root (e.g. via `..`, absolute
//! paths, or symlinks). All tools that touch the filesystem must funnel
//! through this helper so the escape policy stays consistent.

use std::path::{Path, PathBuf};

/// Resolve `path` (absolute or relative) against `root` and require the
/// canonicalised result to live under the canonicalised `root`. Returns an
/// error string suitable for `ErrorData::invalid_params` on rejection.
pub(crate) fn resolve_within(root: &Path, path: &str) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    let candidate: PathBuf = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };

    let root_canon = std::fs::canonicalize(root)
        .map_err(|e| format!("workdir not accessible: {} ({e})", root.display()))?;

    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|e| format!("path not accessible: {} ({e})", candidate.display()))?;

    if !resolved.starts_with(&root_canon) {
        return Err(format!(
            "path escapes workspace: {} not within {}",
            resolved.display(),
            root_canon.display()
        ));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn resolve_within_rejects_escape() {
        let dir = tempdir().expect("tempdir");
        let inside = dir.path().join("file.txt");
        fs::write(&inside, b"x").expect("write");
        // Symlink targeting outside the dir should be rejected.
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join("dev-mcp-paths-escape-target");
            let _ = fs::remove_file(&outside);
            fs::write(&outside, b"y").expect("write outside");
            let link = dir.path().join("link.txt");
            std::os::unix::fs::symlink(&outside, &link).expect("symlink");
            let err = resolve_within(dir.path(), "link.txt").unwrap_err();
            assert!(err.contains("escapes workspace"), "got: {err}");
            let _ = fs::remove_file(&outside);
        }
        // Resolves a normal path inside.
        let p = resolve_within(dir.path(), "file.txt").expect("resolve");
        assert!(p.ends_with("file.txt"));
    }
}
