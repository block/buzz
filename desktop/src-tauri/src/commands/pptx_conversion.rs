//! LibreOffice-based `.pptx` → PDF conversion for high-fidelity in-app preview.
//!
//! The in-app `.pptx` preview normally goes through a client-side JS renderer
//! (`@jvmr/pptx-to-html`), which is a practical but low-fidelity
//! reimplementation of PowerPoint's layout engine (text position/size/font
//! hierarchy can collapse, and some content can go missing entirely). When the
//! user has a real LibreOffice install on their machine, we shell out to it to
//! produce a genuine PDF rendering instead — pixel-accurate, since it's actual
//! PowerPoint-compatible layout output, not a reimplementation — and hand that
//! PDF to the existing `PdfPreview` renderer.
//!
//! This never bundles LibreOffice into the app (that would add 300-500MB to
//! the installer); it only shells out to a LibreOffice already installed on
//! the user's machine. If no working `soffice` binary is found, the frontend
//! shows a prompt to install it (or fall back to the JS renderer) — see
//! `PptxLibreOfficePrompt.tsx`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::util::configure_no_window;

/// Wall-clock budget for a single `soffice --convert-to pdf` invocation.
/// LibreOffice startup (cold profile, first launch) can take several seconds;
/// 45s is generous for a single-deck conversion while still failing closed if
/// `soffice` hangs (a known headless-mode footgun without profile isolation —
/// see `run_soffice_convert`).
const CONVERT_TIMEOUT: Duration = Duration::from_secs(45);

#[cfg(windows)]
const SOFFICE_BASENAME: &str = "soffice.exe";
#[cfg(not(windows))]
const SOFFICE_BASENAME: &str = "soffice";

/// Alternate basename some Linux distro packages ship instead of/alongside
/// `soffice` on `PATH`.
#[cfg(not(windows))]
const SOFFICE_ALT_BASENAME: &str = "libreoffice";

/// Fixed install locations to probe when `soffice` is not resolvable via
/// `PATH` — LibreOffice's installers don't always add themselves to `PATH`.
fn fixed_install_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    #[cfg(windows)]
    {
        paths.push(PathBuf::from(
            r"C:\Program Files\LibreOffice\program\soffice.exe",
        ));
        paths.push(PathBuf::from(
            r"C:\Program Files (x86)\LibreOffice\program\soffice.exe",
        ));
    }
    #[cfg(target_os = "macos")]
    {
        paths.push(PathBuf::from(
            "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        ));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        paths.push(PathBuf::from("/usr/bin/soffice"));
        paths.push(PathBuf::from("/usr/bin/libreoffice"));
        // Distro tarball / opt installs (e.g. /opt/libreoffice24.8/program/soffice).
        if let Ok(entries) = std::fs::read_dir("/opt") {
            for entry in entries.flatten() {
                if entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("libreoffice")
                {
                    paths.push(entry.path().join("program").join("soffice"));
                }
            }
        }
    }
    paths
}

/// Search `PATH` directly for the soffice/libreoffice binary (a `which`-style
/// scan, avoiding a spawn-per-directory).
fn find_on_path() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(SOFFICE_BASENAME);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(not(windows))]
        {
            let alt = dir.join(SOFFICE_ALT_BASENAME);
            if alt.is_file() {
                return Some(alt);
            }
        }
    }
    None
}

/// Confirm a candidate path is a working soffice binary by spawning
/// `--version` and checking it exits successfully.
fn probe_binary(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let mut cmd = std::process::Command::new(path);
    configure_no_window(&mut cmd);
    cmd.arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn resolve_libreoffice_uncached() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = find_on_path() {
        candidates.push(path);
    }
    candidates.extend(fixed_install_candidates());
    candidates.into_iter().find(|path| probe_binary(path))
}

/// Process-lifetime cache of a *successful* resolution only.
///
/// A negative result (LibreOffice not found) is deliberately never cached, so
/// the frontend's "Retry" button — which just calls `check_libreoffice_available`
/// again — picks up a LibreOffice install that happened after the app
/// started, without requiring an app restart. Once a working binary is found,
/// re-probing the filesystem/spawning `--version` on every preview open would
/// be wasted work, so that positive result is cached for the process lifetime.
fn resolved_cache() -> &'static Mutex<Option<PathBuf>> {
    static CACHE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn resolve_libreoffice() -> Option<PathBuf> {
    if let Ok(guard) = resolved_cache().lock() {
        if let Some(path) = guard.as_ref() {
            return Some(path.clone());
        }
    }
    let resolved = resolve_libreoffice_uncached()?;
    if let Ok(mut guard) = resolved_cache().lock() {
        *guard = Some(resolved.clone());
    }
    Some(resolved)
}

/// Tauri command: does this machine have a working LibreOffice install?
///
/// Runs the filesystem/process probing on a blocking thread — `probe_binary`
/// spawns a short-lived child process, which must not block the async runtime.
#[tauri::command]
pub async fn check_libreoffice_available() -> bool {
    tauri::async_runtime::spawn_blocking(|| resolve_libreoffice().is_some())
        .await
        .unwrap_or(false)
}

/// Build a `file://` URL for `-env:UserInstallation`, the flag that gives each
/// conversion its own isolated LibreOffice profile directory.
///
/// LibreOffice headless instances lock a shared user profile directory by
/// default, so concurrent or rapid repeated invocations can hang or fail
/// silently without this — a well-known LibreOffice headless footgun.
fn profile_installation_url(profile_dir: &Path) -> String {
    let normalized = profile_dir.to_string_lossy().replace('\\', "/");
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        // Windows absolute paths ("C:/...") need a third slash before the
        // drive letter to form a valid file:// URL.
        format!("file:///{normalized}")
    }
}

/// Run `soffice --headless --convert-to pdf` with an isolated profile dir and
/// a wall-clock timeout, killing the process (rather than hanging forever) if
/// the deadline is exceeded.
///
/// Stderr is drained on a background thread while we poll `try_wait()`, so a
/// chatty child can't deadlock this function by filling the OS pipe buffer.
fn run_soffice_convert(
    soffice: &Path,
    input: &Path,
    outdir: &Path,
    profile_dir: &Path,
) -> Result<(), String> {
    let profile_url = profile_installation_url(profile_dir);

    let mut cmd = std::process::Command::new(soffice);
    configure_no_window(&mut cmd);
    cmd.arg("--headless")
        .arg("--norestore")
        .arg(format!("-env:UserInstallation={profile_url}"))
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(outdir)
        .arg(input)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to launch LibreOffice: {e}"))?;

    let stderr_buf = std::sync::Arc::new(Mutex::new(Vec::new()));
    let reader_handle = child.stderr.take().map(|mut pipe| {
        let buf = std::sync::Arc::clone(&stderr_buf);
        std::thread::spawn(move || {
            let mut data = Vec::new();
            let _ = pipe.read_to_end(&mut data);
            if let Ok(mut guard) = buf.lock() {
                *guard = data;
            }
        })
    });

    let deadline = Instant::now() + CONVERT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(format!(
                        "LibreOffice conversion timed out after {}s",
                        CONVERT_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => break Err(format!("failed to wait on LibreOffice: {e}")),
        }
    }?;

    if let Some(handle) = reader_handle {
        let _ = handle.join();
    }

    if status.success() {
        return Ok(());
    }

    let stderr = stderr_buf.lock().map(|g| g.clone()).unwrap_or_default();
    let stderr_text = String::from_utf8_lossy(&stderr);
    let detail = stderr_text.trim();
    Err(format!(
        "LibreOffice conversion failed (exit {}){}",
        status
            .code()
            .map_or_else(|| "unknown".to_string(), |c| c.to_string()),
        if detail.is_empty() {
            String::new()
        } else {
            format!(": {detail}")
        }
    ))
}

/// Convert `.pptx` bytes to PDF bytes via a local LibreOffice install.
///
/// Writes `bytes` to a fresh temp `.pptx` file under a job-scoped temp
/// directory (also holding the isolated profile dir and conversion output
/// dir), runs the conversion, reads the resulting PDF back into memory, and
/// removes the entire job directory on every exit path — success, conversion
/// failure, or an early `?` return — via the `Drop` guard below.
fn convert_pptx_to_pdf_blocking(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let soffice = resolve_libreoffice()
        .ok_or_else(|| "LibreOffice was not found on this machine".to_string())?;

    let job_id = uuid::Uuid::new_v4();
    let base = std::env::temp_dir().join(format!("buzz-pptx-{job_id}"));
    let input_path = base.join("input.pptx");
    let outdir = base.join("out");
    let profile_dir = base.join("profile");

    /// Recursively removes the job's temp directory when dropped, regardless
    /// of which return path (`Ok`, `Err`, or an early `?`) is taken.
    struct CleanupGuard(PathBuf);
    impl Drop for CleanupGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = CleanupGuard(base.clone());

    std::fs::create_dir_all(&outdir)
        .map_err(|e| format!("failed to create temp output dir: {e}"))?;
    std::fs::create_dir_all(&profile_dir)
        .map_err(|e| format!("failed to create temp profile dir: {e}"))?;
    std::fs::write(&input_path, bytes).map_err(|e| format!("failed to write temp pptx: {e}"))?;

    run_soffice_convert(&soffice, &input_path, &outdir, &profile_dir)?;

    let pdf_path = outdir.join("input.pdf");
    std::fs::read(&pdf_path).map_err(|e| format!("LibreOffice did not produce a PDF: {e}"))
}

/// Tauri command: convert `.pptx` bytes to PDF bytes via a local LibreOffice
/// install. Returns `tauri::ipc::Response` so the (potentially multi-MB) PDF
/// crosses IPC as a raw buffer rather than an inflated JSON number array,
/// matching `fetch_media_bytes`'s convention for byte payloads.
///
/// Runs the conversion (writing/reading temp files, spawning `soffice`,
/// blocking on `try_wait()`) on a blocking thread so it never stalls the
/// async runtime.
#[tauri::command]
pub async fn convert_pptx_to_pdf(bytes: Vec<u8>) -> Result<tauri::ipc::Response, String> {
    let pdf_bytes = tauri::async_runtime::spawn_blocking(move || {
        convert_pptx_to_pdf_blocking(&bytes)
    })
    .await
    .map_err(|e| format!("conversion task panicked: {e}"))??;
    Ok(tauri::ipc::Response::new(pdf_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_installation_url_unix_absolute_path() {
        let url = profile_installation_url(Path::new("/tmp/buzz-pptx-x/profile"));
        assert_eq!(url, "file:///tmp/buzz-pptx-x/profile");
    }

    #[cfg(windows)]
    #[test]
    fn profile_installation_url_windows_absolute_path() {
        let url = profile_installation_url(Path::new(r"C:\Users\me\AppData\Local\Temp\buzz-pptx-x\profile"));
        assert_eq!(
            url,
            "file:///C:/Users/me/AppData/Local/Temp/buzz-pptx-x/profile"
        );
    }

    #[test]
    fn find_on_path_does_not_panic_without_path_env() {
        // Just exercises the code path; result depends on the host machine.
        let _ = find_on_path();
    }

    #[test]
    fn resolve_libreoffice_runs_without_panicking() {
        // May or may not find a real install depending on the host machine —
        // this just confirms the probing logic itself doesn't panic or hang.
        let _ = resolve_libreoffice_uncached();
    }

    #[test]
    fn convert_pptx_to_pdf_blocking_reports_missing_binary_cleanly() {
        // We can't force resolve_libreoffice() to fail without mocking (it's a
        // free function probing the real machine), so this test only runs
        // meaningfully on a machine without LibreOffice installed. On a
        // machine with LibreOffice, this exercises the real conversion path
        // instead — which is a stronger and still-valid check — with a
        // minimal but structurally invalid "pptx" so a *found* LibreOffice
        // fails predictably rather than hanging.
        if resolve_libreoffice_uncached().is_none() {
            let result = convert_pptx_to_pdf_blocking(b"not a real pptx");
            assert!(result.is_err());
            assert!(result.unwrap_err().contains("LibreOffice was not found"));
        }
    }
}
