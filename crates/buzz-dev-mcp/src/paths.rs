//! Path resolution and file I/O shared across dev-mcp tools.
//!
//! `resolve_path` resolves and canonicalizes a user-supplied path against a
//! workspace root. A leading `~` expands to the user's home directory (bare
//! `~` or `~/...`), matching the shell tool. No containment enforcement — the
//! resolved path may land anywhere on the filesystem (consistent with the
//! `shell` tool's posture).
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

    // Expand a leading `~` (bare or `~/...`) to the user's home directory,
    // matching the shell tool's tilde semantics. Without this, a user-named
    // path like `~/.claude/skills/x` takes the relative branch and resolves
    // under the workspace root (`<root>/~/.claude/...`), which never exists.
    // We deliberately do NOT handle `~user` (another user's home): that needs
    // a passwd lookup and is out of scope, mirroring the conservative posture
    // for un-mappable MSYS forms above. `~user...` falls through untouched and
    // fails with the clear `path not accessible` error rather than mis-mapping.
    let expanded = expand_tilde(path, home_dir().as_deref());
    let path: &str = expanded.as_deref().unwrap_or(path);

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

/// Expand a leading `~` to the user's home directory, returning `Some(expanded)`
/// when a rewrite happened and `None` when the input should be used unchanged.
///
/// Handles the two shell forms that map deterministically to a home directory:
///   - bare `~`            -> `home`
///   - `~/rest` (or `~\rest` on Windows) -> `<home>/rest`
///
/// A leading `~` followed by anything else (`~user`, `~+`, `~foo`) is a form we
/// cannot resolve without extra state, so it is left untouched — consistent with
/// how `msys_to_windows` leaves un-mappable inputs alone. Returns `None` when
/// `home` is `None` (unset) so the caller falls back to the raw path. Kept pure
/// (home passed in) so it is testable without mutating process environment.
fn expand_tilde(path: &str, home: Option<&str>) -> Option<String> {
    let rest = path.strip_prefix('~')?;
    // Only a bare `~` or a `~` immediately followed by a path separator is a
    // home-relative reference. Anything else (`~user`) is left to the caller.
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    if !rest.is_empty() && !rest.starts_with(is_sep) {
        return None;
    }

    let home = home?;
    if home.is_empty() {
        return None;
    }

    if rest.is_empty() {
        // Bare `~` -> home directory.
        return Some(home.to_string());
    }
    // `~/rest` -> `<home>/rest`. `rest` begins with a separator, so strip it to
    // avoid an absolute-looking join and let `Path` re-add the separator.
    let tail = rest.trim_start_matches(is_sep);
    let joined = Path::new(home).join(tail);
    Some(joined.to_string_lossy().into_owned())
}

/// The user's home directory from the environment. Reads `$HOME` first, falling
/// back to `%USERPROFILE%` on Windows, then hands the raw values to `select_home`
/// (pure, so it is testable without mutating process env). Returns `None` if no
/// usable value is set or the value is not UTF-8.
fn home_dir() -> Option<String> {
    let home = std::env::var_os("HOME").and_then(|v| v.into_string().ok());
    #[cfg(windows)]
    let userprofile = std::env::var_os("USERPROFILE").and_then(|v| v.into_string().ok());
    #[cfg(not(windows))]
    let userprofile: Option<String> = None;
    select_home(home.as_deref(), userprofile.as_deref())
}

/// Choose the home directory from the two env candidates, preferring `$HOME`.
///
/// `$HOME` is preferred because that is exactly what bash — and therefore the
/// `shell` tool — expands `~` against, and `HOME` is passed through to the MCP
/// child on every platform (see `buzz-agent`'s `PASSTHROUGH_ENV`). Picking
/// `USERPROFILE` first on Windows would diverge from the shell tool whenever the
/// two differ (a git-bash `HOME=/c/Users/x`, or an `mcpServers[].env` override),
/// which is precisely the "match the shell tool" contract this fix exists for.
/// `USERPROFILE` is only a Windows fallback for when `HOME` is unset.
///
/// On Windows the chosen value is passed through `msys_to_windows` so an MSYS
/// `HOME` (`/c/Users/x`) becomes a native path (`C:\Users\x`) — `~` expansion
/// happens after `msys_to_windows` in `resolve_path`, so the spliced-in home
/// would otherwise never be translated and `canonicalize` would reject it. An
/// MSYS form with no Windows equivalent (`/home/x`) falls through untranslated
/// and fails with the clear `path not accessible` error, the correct outcome.
/// Empty strings are treated as unset.
fn select_home(home: Option<&str>, userprofile: Option<&str>) -> Option<String> {
    fn non_empty(v: Option<&str>) -> Option<&str> {
        v.filter(|s| !s.is_empty())
    }
    let chosen = non_empty(home).or_else(|| non_empty(userprofile))?;
    #[cfg(windows)]
    {
        Some(msys_to_windows(chosen))
    }
    #[cfg(not(windows))]
    {
        Some(chosen.to_string())
    }
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
/// MSYS install root (from the host's Git for Windows install), which this
/// process does not reliably know. We deliberately do NOT guess it: such a path
/// falls through untranslated and fails with the clear `path not accessible`
/// error rather than being silently mis-mapped to the wrong location. Resolving
/// it correctly would require shelling out to `cygpath`; that is out of scope
/// here and these paths are not a normal target for agent file I/O.
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

    // `expand_tilde` is pure (home is passed in), so these cases need no env
    // mutation and cannot race parallel tests.
    #[test]
    fn expand_tilde_forms() {
        let home = "/home/agent";

        // Non-tilde inputs are never rewritten.
        assert_eq!(expand_tilde("file.txt", Some(home)), None);
        assert_eq!(expand_tilde("/abs/path", Some(home)), None);
        assert_eq!(expand_tilde("sub/~notleading", Some(home)), None);

        // `~user` and other non-separator suffixes are left for the caller.
        assert_eq!(expand_tilde("~user/x", Some(home)), None);
        assert_eq!(expand_tilde("~foo", Some(home)), None);

        // Bare `~` and `~/rest` expand against the supplied home.
        assert_eq!(expand_tilde("~", Some(home)), Some(home.to_string()));
        let expanded = expand_tilde("~/.claude/skills/x", Some(home)).expect("expands");
        assert_eq!(
            expanded,
            Path::new(home).join(".claude/skills/x").to_string_lossy()
        );

        // Unset or empty home -> no rewrite, caller falls back to the raw path.
        assert_eq!(expand_tilde("~/rest", None), None);
        assert_eq!(expand_tilde("~", None), None);
        assert_eq!(expand_tilde("~/rest", Some("")), None);
    }

    // `select_home` is pure (both env candidates passed in), so it exercises the
    // HOME-first preference and empty/unset handling without mutating process
    // env or racing parallel tests. `select_home` itself does not gate the
    // fallback by platform — `home_dir` is what only supplies `userprofile` on
    // Windows — so these assertions hold identically on every platform.
    #[test]
    fn select_home_prefers_home() {
        // $HOME wins when both are set.
        assert_eq!(
            select_home(Some("/home/agent"), Some("/other")),
            Some("/home/agent".to_string())
        );
        // Empty $HOME is treated as unset -> fall back to the second candidate.
        assert_eq!(
            select_home(Some(""), Some("/other")),
            Some("/other".to_string())
        );
        // No usable candidate -> None.
        assert_eq!(select_home(None, None), None);
        assert_eq!(select_home(Some(""), Some("")), None);
    }

    // End-to-end through `resolve_path`, exercising the real `home_dir()` env
    // read: a `~/...` path resolves against the actual home directory, not the
    // workspace root. Uses a temp file created under the real home so it does
    // not mutate the environment.
    #[test]
    fn resolve_path_expands_tilde_against_home() {
        let home = match home_dir() {
            Some(h) if !h.is_empty() => h,
            _ => return, // No home in this environment (e.g. minimal CI) — skip.
        };
        let marker = format!(".dev-mcp-tilde-test-{}", std::process::id());
        let target = Path::new(&home).join(&marker);
        fs::write(&target, b"z").expect("write under home");

        let workspace = tempdir().expect("tempdir");
        let resolved = resolve_path(workspace.path(), &format!("~/{marker}"))
            .expect("tilde path resolves against home, not workspace");
        let want = std::fs::canonicalize(&target).expect("canon");
        assert_eq!(resolved, want);

        let _ = fs::remove_file(&target);
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

        // Windows `select_home` behavior: HOME still wins over USERPROFILE, and
        // an MSYS-form HOME is translated to a native path so the value spliced
        // in during `~` expansion (which runs after `msys_to_windows`) resolves.
        // Both candidates are passed in, so this needs no process-env mutation
        // and does not silently no-op the way a real-env read would when HOME is
        // unset on CI.
        #[test]
        fn select_home_translates_msys_home_and_prefers_it() {
            // Divergent HOME/USERPROFILE: HOME wins, and its MSYS cygdrive form
            // is translated to the native path so canonicalize can use it.
            assert_eq!(
                select_home(Some("/c/Users/agent"), Some(r"C:\Users\other")),
                Some(r"C:\Users\agent".to_string())
            );
            // A native-form HOME is preferred and passes through unchanged.
            assert_eq!(
                select_home(Some(r"C:\Users\agent"), Some(r"C:\Users\other")),
                Some(r"C:\Users\agent".to_string())
            );
            // HOME unset -> fall back to USERPROFILE (already native).
            assert_eq!(
                select_home(None, Some(r"C:\Users\other")),
                Some(r"C:\Users\other".to_string())
            );
            // An MSYS HOME with no Windows equivalent (`/home/x`) is left
            // untranslated; it fails downstream with a clear error rather than
            // being mis-mapped — the intended conservative outcome.
            assert_eq!(
                select_home(Some("/home/agent"), None),
                Some("/home/agent".to_string())
            );
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
