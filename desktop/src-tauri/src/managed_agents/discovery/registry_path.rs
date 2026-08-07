//! Windows fallbacks for command resolution: PATH shim scanning and the
//! registry `Environment\Path` fallback.
//!
//! The inherited process `PATH` can be a stale snapshot: apps relaunched by
//! an updater, services, and long-lived parents keep the environment they
//! were launched with, so PATH entries written to the registry later (e.g. a
//! freshly installed agent CLI) stay invisible even though a newly spawned
//! process would see them. Reading `Environment\Path` from both hives
//! restores the authoritative post-login PATH and mirrors the resolver's own
//! `git_bash_from_registry` fallback.

#[cfg(windows)]
use std::path::PathBuf;

/// Windows-only resolution steps run after the process-env `.exe` scan.
///
/// 1. Scan the process PATH for `.cmd`/`.bat` shims (npm globals).
/// 2. Fall back to the machine and per-user registry `Path` values.
#[cfg(windows)]
pub(super) fn resolve_windows_fallbacks(basenames: &[String]) -> Option<PathBuf> {
    for basename in basenames.iter().skip(1) {
        for candidate in path_candidates_from_env_raw(basename) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    resolve_via_registry_path(basenames)
}

/// Like `path_candidates_from_env` but joins `basename` as-is (no `.exe`
/// suffix). Used for `.cmd`/`.bat` shim resolution on Windows.
#[cfg(windows)]
fn path_candidates_from_env_raw(basename: &str) -> Vec<PathBuf> {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(basename))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Resolve against the machine and per-user registry `Path` values.
///
/// Registry values are REG_EXPAND_SZ; `%VAR%` references are resolved so
/// entries like `%SystemRoot%\system32` become real directories. On
/// expansion failure, the raw value is skipped.
#[cfg(windows)]
fn resolve_via_registry_path(basenames: &[String]) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ,
    };

    const VALUE: &str = "Path";
    let value: Vec<u16> = VALUE.encode_utf16().chain(Some(0)).collect();

    // Machine value first, then per-user — matches the effective merge order
    // of the process environment (`HKLM` entries precede `HKCU` entries).
    for (hive, key) in [
        (
            HKEY_LOCAL_MACHINE,
            "SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
        ),
        (HKEY_CURRENT_USER, "Environment"),
    ] {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();

        // SAFETY: key/value are null-terminated UTF-16 for the duration of
        // each call, and every successfully opened handle is closed before
        // the next hive is tried.
        unsafe {
            let mut handle = std::ptr::null_mut();
            if RegOpenKeyExW(hive, key.as_ptr(), 0, KEY_READ, &mut handle) != ERROR_SUCCESS {
                continue;
            }

            let mut byte_len = 0;
            let status = RegQueryValueExW(
                handle,
                value.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut byte_len,
            );
            if (status != ERROR_SUCCESS && status != ERROR_MORE_DATA) || byte_len == 0 {
                RegCloseKey(handle);
                continue;
            }

            let mut data = vec![0u16; (byte_len as usize).div_ceil(2)];
            let status = RegQueryValueExW(
                handle,
                value.as_ptr(),
                std::ptr::null(),
                std::ptr::null_mut(),
                data.as_mut_ptr().cast(),
                &mut byte_len,
            );
            RegCloseKey(handle);
            if status != ERROR_SUCCESS {
                continue;
            }

            while data.last() == Some(&0) {
                data.pop();
            }
            let raw = OsString::from_wide(&data);

            let expanded: Vec<u16> = raw.encode_wide().chain(Some(0)).collect();
            let mut buf = vec![0u16; 1024];
            let expanded_len =
                ExpandEnvironmentStringsW(expanded.as_ptr(), buf.as_mut_ptr(), buf.len() as u32);
            if expanded_len == 0 {
                continue;
            }
            if expanded_len > buf.len() as u32 {
                buf.resize(expanded_len as usize, 0);
                let _ = ExpandEnvironmentStringsW(
                    expanded.as_ptr(),
                    buf.as_mut_ptr(),
                    buf.len() as u32,
                );
            }
            while buf.last() == Some(&0) {
                buf.pop();
            }
            let expanded = OsString::from_wide(&buf);

            if let Some(path) = resolve_in_dirs(basenames, std::env::split_paths(&expanded)) {
                return Some(path);
            }
        }
    }

    None
}

/// Scan `dirs` for any basename in `basenames` that exists as a file.
///
/// Extracted from the registry fallback so the dir-scanning logic is
/// testable without touching the real registry.
#[cfg(windows)]
pub(super) fn resolve_in_dirs(
    basenames: &[String],
    dirs: impl Iterator<Item = PathBuf>,
) -> Option<PathBuf> {
    for dir in dirs {
        for basename in basenames {
            let candidate = dir.join(basename);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
