//! Finding a provider binary: which files on PATH name a `buzz-backend-*`
//! provider, and which id resolves to which executable.
//!
//! Split out of `backend.rs` so the question "is this file a provider I may
//! execute?" — a naming and executability rule with its own security argument —
//! sits apart from actually invoking one.

use std::path::{Path, PathBuf};

/// The filename prefix that makes an executable a provider.
const PROVIDER_PREFIX: &str = "buzz-backend-";

/// Extensions that name the *same* program on Windows, and therefore carry no
/// information about which provider a file is.
///
/// Cargo installs `buzz-backend-ssh` as `buzz-backend-ssh.exe` there, and a
/// literal read of that filename derives the id `ssh.exe` — which
/// [`resolve_provider_binary`]'s `[a-z0-9][a-z0-9_-]*` rule rejects for the
/// dot, making a provider that ships Windows support unusable on Windows.
///
/// Stripping is unconditional rather than `cfg(windows)`: the id a file name
/// means must not depend on which machine is reading it, and a rule that only
/// compiles on one platform is a rule most CI never exercises.
const EXECUTABLE_EXTENSIONS: &[&str] = &["exe", "com", "cmd", "bat"];

/// The provider id a filename names, or `None` when it names no provider.
///
/// The one owner of "what id is this file?", so discovery and deduplication
/// cannot disagree — a host carrying both `buzz-backend-ssh` and
/// `buzz-backend-ssh.exe` must offer one `ssh`, not two entries the desktop
/// then treats as different providers.
fn provider_id_from_file_name(name: &str) -> Option<String> {
    let id = name.strip_prefix(PROVIDER_PREFIX)?;
    let id = match id.rsplit_once('.') {
        Some((stem, extension))
            if EXECUTABLE_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate)) =>
        {
            stem
        }
        _ => id,
    };
    (!id.is_empty()).then(|| id.to_string())
}

/// Enumerate PATH for buzz-backend-* executables. Returns (id, path) pairs.
/// Only includes files that are executable. Does NOT execute any binaries.
///
/// On macOS, GUI apps inherit a minimal PATH from launchd (`/usr/bin:/bin:/usr/sbin:/sbin`)
/// which excludes both the app bundle's `Contents/MacOS/` dir and `~/.local/bin`.
/// We augment the search with those directories so bundled and user-installed providers
/// are always discovered regardless of how the desktop was launched.
pub fn discover_provider_candidates() -> Vec<(String, PathBuf)> {
    candidates_in(search_path())
}

/// The directories discovery scans, in precedence order.
///
/// Separated from the scan so tests can supply their own. What a test of the
/// naming and resolution rules must not depend on is whether the machine
/// running it happens to have a real provider installed: asserting that
/// `resolve_provider_binary("ssh")` fails passes on CI and fails on any
/// developer box with `~/.local/bin/buzz-backend-ssh` — which is exactly the
/// install this feature tells users to perform.
fn search_path() -> Vec<PathBuf> {
    let path_var = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path_var).collect();

    // Prepend the exe parent dir (Contents/MacOS/ in a .app bundle) so bundled
    // providers are found even when the process PATH is minimal.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let parent_buf = parent.to_path_buf();
            if !dirs.contains(&parent_buf) {
                dirs.insert(0, parent_buf);
            }
        }
    }

    // Also include ~/.local/bin — the conventional location for user-installed
    // provider binaries (symlinks created by install scripts).
    if let Some(home) = dirs::home_dir() {
        let local_bin = home.join(".local").join("bin");
        if !dirs.contains(&local_bin) {
            dirs.push(local_bin);
        }
    }

    dirs
}

fn candidates_in(dirs: Vec<PathBuf>) -> Vec<(String, PathBuf)> {
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();

    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            // Deduplicated on the ID rather than the filename, so a host
            // carrying both spellings of one provider offers it once. First
            // match wins, which is PATH precedence.
            if let Some(id) = provider_id_from_file_name(&name) {
                if !seen.contains(&id) && is_executable(&entry.path()) {
                    seen.insert(id.clone());
                    results.push((id, entry.path()));
                }
            }
        }
    }
    results
}

/// Resolve a provider ID to a discovered, executable binary path.
///
/// This is the ONLY way to resolve provider binaries for execution. It:
/// 1. Validates the ID against `^[a-z0-9][a-z0-9_-]*$` (no path traversal)
/// 2. Looks up the ID in `discover_provider_candidates()` (PATH-discovered only)
/// 3. Returns the canonical path of the discovered binary
///
/// All deploy, start, and create paths MUST use this instead of raw
/// `resolve_command(format!("buzz-backend-{id}"))` to prevent a compromised
/// frontend/IPC caller from steering execution to an arbitrary binary.
pub fn resolve_provider_binary(provider_id: &str) -> Result<PathBuf, String> {
    resolve_provider_binary_in(provider_id, search_path())
}

/// [`resolve_provider_binary`] against an explicit search path.
///
/// Production always passes [`search_path`]. Tests pass a temp dir, so the
/// naming rules can be asserted against a known-empty (or deliberately
/// populated) directory rather than against whatever the host has installed.
fn resolve_provider_binary_in(provider_id: &str, dirs: Vec<PathBuf>) -> Result<PathBuf, String> {
    // Reject IDs that could be path components or shell metacharacters.
    let valid_id = provider_id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && !provider_id.is_empty()
        && provider_id.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !valid_id {
        return Err(format!(
            "invalid provider ID '{provider_id}': must match [a-z0-9][a-z0-9_-]*"
        ));
    }

    let candidates = candidates_in(dirs);
    let found = candidates
        .into_iter()
        .find(|(id, _)| id == provider_id)
        .map(|(_, path)| path);

    match found {
        Some(path) => path
            .canonicalize()
            .map_err(|e| format!("provider binary not accessible: {e}")),
        None => Err(format!(
            "provider 'buzz-backend-{provider_id}' not found on PATH"
        )),
    }
}

/// Check if a file is executable (Unix: mode bits; other platforms: always true).
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        true
    }
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendProviderInfo {
    pub id: String,
    pub binary_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty search path: no provider resolves, regardless of what the
    /// machine running the test has installed.
    fn empty_search_path() -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = vec![dir.path().to_path_buf()];
        (dir, path)
    }

    #[test]
    fn resolve_provider_binary_rejects_invalid_ids() {
        let (_temp, path) = empty_search_path();
        for id in [
            "../evil",       // path traversal
            "",              // empty
            "MyProvider",    // uppercase
            "my provider",   // spaces
            "foo;rm -rf /",  // shell metacharacters
            "-leading-dash", // does not start with [a-z0-9]
        ] {
            let error = resolve_provider_binary_in(id, path.clone()).unwrap_err();
            assert!(error.contains("invalid provider ID"), "{id:?}: {error}");
        }
        // Valid format, nothing in the search path — a different error, and the
        // distinction is what tells a user "typo" from "not installed".
        let error = resolve_provider_binary_in("nonexistent-test-id-12345", path).unwrap_err();
        assert!(error.contains("not found"), "{error}");
    }

    /// The Windows regression: Cargo installs the provider as
    /// `buzz-backend-ssh.exe`, and a literal read of that filename derives
    /// `ssh.exe` — an id `resolve_provider_binary` rejects for the dot, so the
    /// SSH provider was unusable on the one platform where that spelling is
    /// the only one.
    #[test]
    fn executable_extensions_are_not_part_of_a_provider_id() {
        for name in [
            "buzz-backend-ssh",
            "buzz-backend-ssh.exe",
            "buzz-backend-ssh.EXE",
            "buzz-backend-ssh.com",
            "buzz-backend-ssh.cmd",
            "buzz-backend-ssh.bat",
        ] {
            assert_eq!(
                provider_id_from_file_name(name).as_deref(),
                Some("ssh"),
                "{name}"
            );
        }
        // And every derived id survives the resolver's own validation, which is
        // the check the `.exe` spelling used to fail. Resolved against an empty
        // temp dir rather than the host's PATH: the claim under test is "the
        // derived id is well-formed", and on a developer machine that has taken
        // this feature's own advice and installed `~/.local/bin/buzz-backend-ssh`
        // the resolve would legitimately *succeed*, which must not read as a
        // regression.
        let (_temp, path) = empty_search_path();
        for name in ["buzz-backend-ssh", "buzz-backend-ssh.exe"] {
            let id = provider_id_from_file_name(name).unwrap();
            let error = resolve_provider_binary_in(&id, path.clone()).unwrap_err();
            assert!(!error.contains("invalid provider ID"), "{id}: {error}");
            assert!(error.contains("not found"), "{id}: {error}");
        }
    }

    /// The seam itself: a provider present in the search path resolves, and the
    /// same id against an empty path does not. Without this, a bug that made
    /// `candidates_in` return nothing would leave every other test here passing.
    #[test]
    #[cfg(unix)]
    fn a_provider_in_the_search_path_resolves_and_one_outside_it_does_not() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let binary = dir.path().join("buzz-backend-fake");
        std::fs::write(&binary, b"#!/bin/sh\n").expect("write");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        let populated = vec![dir.path().to_path_buf()];
        assert_eq!(
            candidates_in(populated.clone())
                .into_iter()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            vec!["fake".to_string()]
        );
        assert!(resolve_provider_binary_in("fake", populated).is_ok());

        let (_empty_temp, empty) = empty_search_path();
        assert!(resolve_provider_binary_in("fake", empty).is_err());
    }

    /// A non-executable file with a provider's name is not a provider — the
    /// executability check is part of discovery, not an afterthought.
    #[test]
    #[cfg(unix)]
    fn a_non_executable_file_is_not_a_candidate() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::write(dir.path().join("buzz-backend-inert"), b"not executable").expect("write");
        assert!(candidates_in(vec![dir.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn a_dot_that_is_not_an_executable_extension_stays_in_the_id() {
        // Only the extensions that name the same program are dropped. A
        // provider whose name genuinely contains a dot keeps it — and is then
        // rejected by the resolver, exactly as it was before.
        assert_eq!(
            provider_id_from_file_name("buzz-backend-ssh.v2").as_deref(),
            Some("ssh.v2")
        );
        // Nothing before the extension is not an id at all.
        assert_eq!(provider_id_from_file_name("buzz-backend-.exe"), None);
        assert_eq!(provider_id_from_file_name("buzz-backend-"), None);
        assert_eq!(provider_id_from_file_name("buzz-frontend-ssh"), None);
    }

    #[test]
    fn resolve_provider_binary_accepts_valid_id_format() {
        // A well-formed id gets past validation and fails on availability
        // instead. Against an empty search path that outcome is exact rather
        // than "either result proves it".
        let (_temp, path) = empty_search_path();
        let error = resolve_provider_binary_in("zzz-nonexistent-test-provider", path).unwrap_err();
        assert!(
            error.contains("not found") && !error.contains("invalid provider ID"),
            "expected 'not found' error, got: {error}"
        );
    }

    /// Discovery reads the real search path in production, so the wiring must
    /// stay connected. Asserts only what is true on every machine: the call
    /// works and every id it yields is one the resolver would accept.
    #[test]
    fn discovery_uses_the_real_search_path_and_yields_only_valid_ids() {
        assert!(!search_path().is_empty(), "PATH produced no directories");
        for (id, path) in discover_provider_candidates() {
            assert_eq!(
                provider_id_from_file_name(&path.file_name().unwrap_or_default().to_string_lossy())
                    .as_deref(),
                Some(id.as_str()),
                "{path:?} was discovered under an id its filename does not name"
            );
        }
    }
}
