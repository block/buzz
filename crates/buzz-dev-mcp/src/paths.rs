//! Path resolution and file I/O shared across dev-mcp tools.
//!
//! `resolve_path` resolves and canonicalizes a user-supplied path against a
//! workspace root. No containment enforcement — the resolved path may land
//! anywhere on the filesystem (consistent with the `shell` tool's posture).
//!
//! `read_text_file` builds on `resolve_path` to provide the full
//! resolve → stat → size-check → read → UTF-8 decode pipeline shared by
//! `read_file` and `str_replace`.

use crate::shell::SharedState;
use rmcp::ErrorData;
use std::path::{Path, PathBuf};

pub(crate) const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;

/// Resolve `path` (absolute or relative) against `root` and canonicalize
/// the result. Returns an error string suitable for `ErrorData::invalid_params`
/// if the path cannot be resolved.
pub(crate) fn resolve_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    // The agent runs inside MSYS bash and naturally hands us MSYS-form absolute
    // paths (`/c/Users/...`). On Windows those are NOT `is_absolute()` (a leading
    // `/` has no drive `Prefix`), so without translation they'd take the relative
    // branch and `root.join` would double the drive (`C:/c/Users/...`) and fail.
    // The `shell` tool avoids this because bash translates internally; the MCP
    // file tools call `canonicalize` directly, so we mirror that translation here
    // to keep the same posture. No-op on the already-resolved path on Unix.
    #[cfg(windows)]
    let path = &msys_to_windows(path);

    let raw = Path::new(path);
    let candidate: PathBuf = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    };

    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|e| format!("path not accessible: {} ({e})", candidate.display()))?;

    Ok(resolved)
}

/// Translate the MSYS/Cygwin absolute path forms bash would accept into a
/// native Windows path, matching `cygpath -w` semantics so the file tools
/// resolve the same inputs the `shell` tool does. Anything that is not a
/// recognized MSYS-absolute form is returned unchanged.
///
/// Two forms are translated natively because they are deterministic with no
/// external state:
///   - cygdrive: `/c/Users/x` -> `C:\Users\x` (the form that bit the agent).
///   - UNC:      `//server/share/x` -> `\\server\share\x`.
///
/// A third form — root-anchored `/tmp`, `/usr/...`, `/bin` — maps under the
/// MSYS install root (the bundled `git-bash` dir), which this process does not
/// reliably know. We deliberately do NOT guess it: such a path falls through
/// untranslated and fails with the clear `path not accessible` error rather than
/// being silently mis-mapped to the wrong location. Resolving it correctly would
/// require shelling out to the bundled `cygpath`; that is out of scope here and
/// these paths are not a normal target for agent file I/O.
#[cfg(windows)]
fn msys_to_windows(path: &str) -> String {
    // UNC: exactly two leading slashes then a non-empty host segment.
    if let Some(rest) = path.strip_prefix("//") {
        if !rest.is_empty() && !rest.starts_with('/') {
            return format!(r"\\{}", rest.replace('/', r"\"));
        }
        return path.to_string();
    }

    // cygdrive: `/<letter>` optionally followed by `/...`. The drive segment is
    // a single ASCII letter; `/cc/...` (two letters) is a root-anchored path,
    // not a drive, and must NOT match.
    if let Some(rest) = path.strip_prefix('/') {
        let mut chars = rest.chars();
        if let Some(drive) = chars.next() {
            if drive.is_ascii_alphabetic() {
                let after = chars.as_str();
                if after.is_empty() {
                    // `/c` -> `C:\`
                    return format!(r"{}:\", drive.to_ascii_uppercase());
                }
                if let Some(tail) = after.strip_prefix('/') {
                    // `/c/Users/x` -> `C:\Users\x`
                    return format!(
                        r"{}:\{}",
                        drive.to_ascii_uppercase(),
                        tail.replace('/', r"\")
                    );
                }
            }
        }
    }

    // Root-anchored (`/tmp`, `/usr/...`) or anything else: leave untouched.
    path.to_string()
}

/// Resolve a user-supplied path within the workspace, read the file, and
/// return `(resolved_path, utf8_content)`. Rejects files that are not
/// regular files, exceed `MAX_FILE_BYTES`, or are not valid UTF-8.
pub(crate) fn read_text_file(
    state: &SharedState,
    path: &str,
    workdir: Option<&str>,
) -> Result<(PathBuf, String), ErrorData> {
    let workspace_root: PathBuf = match workdir {
        Some(w) => PathBuf::from(w),
        None => state.cwd.clone(),
    };
    let target = match resolve_path(&workspace_root, path) {
        Ok(t) => t,
        Err(e) => return Err(ErrorData::invalid_params(e, None)),
    };

    let meta = match std::fs::metadata(&target) {
        Ok(m) => m,
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("cannot stat {}: {e}", target.display()),
                None,
            ));
        }
    };
    if !meta.is_file() {
        return Err(ErrorData::invalid_params(
            format!("not a regular file: {}", target.display()),
            None,
        ));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(ErrorData::invalid_params(
            format!(
                "file too large: {} is {} bytes (limit {} bytes)",
                target.display(),
                meta.len(),
                MAX_FILE_BYTES
            ),
            None,
        ));
    }

    let file = match std::fs::File::open(&target) {
        Ok(f) => f,
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("cannot open {}: {e}", target.display()),
                None,
            ));
        }
    };
    let mut buf = Vec::with_capacity(meta.len() as usize);
    use std::io::Read;
    match file.take(MAX_FILE_BYTES + 1).read_to_end(&mut buf) {
        Ok(n) if n as u64 > MAX_FILE_BYTES => {
            return Err(ErrorData::invalid_params(
                format!("file grew past {} bytes during read", MAX_FILE_BYTES),
                None,
            ));
        }
        Ok(_) => {}
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("cannot read {}: {e}", target.display()),
                None,
            ));
        }
    }
    let content = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(e) => {
            return Err(ErrorData::internal_error(
                format!("not valid UTF-8: {}: {e}", target.display()),
                None,
            ));
        }
    };

    Ok((target, content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn resolve_path_allows_outside_workspace() {
        let dir = tempdir().expect("tempdir");
        let inside = dir.path().join("file.txt");
        fs::write(&inside, b"x").expect("write");
        // Symlink targeting outside the dir should now resolve successfully.
        #[cfg(unix)]
        {
            let outside = std::env::temp_dir().join("dev-mcp-paths-escape-target");
            let _ = fs::remove_file(&outside);
            fs::write(&outside, b"y").expect("write outside");
            let link = dir.path().join("link.txt");
            std::os::unix::fs::symlink(&outside, &link).expect("symlink");
            let resolved = resolve_path(dir.path(), "link.txt").expect("resolve");
            let outside_canon = std::fs::canonicalize(&outside).expect("canonicalize");
            assert_eq!(resolved, outside_canon);
            let _ = fs::remove_file(&outside);
        }
        // Resolves a normal path inside.
        let p = resolve_path(dir.path(), "file.txt").expect("resolve");
        assert!(p.ends_with("file.txt"));
    }

    // Windows MSYS-absolute path translation. These test `msys_to_windows`
    // directly (the pure rewrite) rather than `resolve_path`, because the latter
    // canonicalizes against the real filesystem and we want deterministic
    // assertions that don't depend on `C:\Users\x` existing on the runner.
    #[cfg(windows)]
    mod windows_msys {
        use super::super::*;
        use std::path::Path;

        #[test]
        fn cygdrive_path_becomes_drive_letter() {
            assert_eq!(msys_to_windows("/c/Users/x"), r"C:\Users\x");
            assert_eq!(msys_to_windows("/d/a/_temp/repo"), r"D:\a\_temp\repo");
        }

        #[test]
        fn cygdrive_root_becomes_drive_root() {
            assert_eq!(msys_to_windows("/c"), r"C:\");
        }

        #[test]
        fn unc_path_becomes_backslash_unc() {
            assert_eq!(msys_to_windows("//server/share/x"), r"\\server\share\x");
        }

        #[test]
        fn windows_absolute_passes_through_unchanged() {
            // Already a native Windows path — must not be mangled.
            assert_eq!(msys_to_windows(r"C:\Users\x"), r"C:\Users\x");
        }

        #[test]
        fn relative_path_passes_through_unchanged() {
            // No leading slash — left for the caller's `root.join`.
            assert_eq!(msys_to_windows("file.txt"), "file.txt");
            assert_eq!(msys_to_windows("sub/file.txt"), "sub/file.txt");
        }

        #[test]
        fn root_anchored_msys_path_is_left_untranslated() {
            // Form 3: maps under the MSYS install root we don't know — must NOT
            // be guessed. Returned unchanged so it fails cleanly downstream
            // rather than being silently mis-mapped.
            assert_eq!(msys_to_windows("/tmp/scratch"), "/tmp/scratch");
            assert_eq!(msys_to_windows("/usr/bin/git"), "/usr/bin/git");
            // Two-letter leading segment is root-anchored, not a drive.
            assert_eq!(msys_to_windows("/cc/x"), "/cc/x");
        }

        #[test]
        fn degenerate_slash_inputs_are_left_untranslated() {
            assert_eq!(msys_to_windows("/"), "/");
            assert_eq!(msys_to_windows("//"), "//");
        }

        #[test]
        fn cygdrive_path_resolves_against_drive_not_doubled() {
            // Integration: a cygdrive path resolves to a real Windows-absolute
            // candidate (the system root always exists), proving the drive is
            // not doubled into C:/c/... as the pre-fix bug did.
            let resolved = resolve_path(Path::new(r"C:\does\not\matter"), "/c/Windows")
                .expect("cygdrive path resolves");
            assert!(resolved
                .to_string_lossy()
                .to_lowercase()
                .contains(r"c:\windows"));
        }
    }
}
