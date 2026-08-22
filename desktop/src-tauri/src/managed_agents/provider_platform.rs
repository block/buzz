// Platform-specific provider discovery and signature policy.
#[cfg(any(windows, test))]
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Derive a provider ID from a staged or installed provider filename.
pub(super) fn provider_id_from_filename(name: &str) -> Option<&str> {
    let raw = name.strip_prefix("buzz-backend-")?;
    let id = [".exe", ".bat", ".cmd"]
        .into_iter()
        .find_map(|extension| {
            raw.get(raw.len().saturating_sub(extension.len())..)
                .filter(|suffix| suffix.eq_ignore_ascii_case(extension))
                .map(|_| &raw[..raw.len() - extension.len()])
        })
        .unwrap_or(raw);

    (!id.is_empty()).then_some(id)
}

/// Add stable platform-owned provider locations ahead of inherited PATH.
pub(super) fn augment_provider_search_dirs(dirs: &mut Vec<PathBuf>) {
    #[cfg(windows)]
    if let Some(provider_dir) = windows_user_provider_dir(std::env::var_os("LOCALAPPDATA")) {
        if !dirs.contains(&provider_dir) {
            dirs.insert(0, provider_dir);
        }
    }

    #[cfg(not(windows))]
    let _ = dirs;
}

#[cfg(any(windows, test))]
fn windows_user_provider_dir(local_app_data: Option<OsString>) -> Option<PathBuf> {
    local_app_data
        .map(PathBuf::from)
        .map(|base| base.join("Buzz").join("providers"))
}

/// Enforce the build-pinned Windows signer policy for immutable staged bytes.
#[cfg(windows)]
pub(super) fn verify_provider_platform_signature(binary: &Path) -> Result<(), String> {
    let configured = option_env!("BUZZ_TRUSTED_PROVIDER_SIGNER_SUBJECTS").unwrap_or("");
    if configured.split(';').map(str::trim).all(str::is_empty) {
        return Ok(());
    }
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or_else(|| {
            "Windows system root is unavailable for provider verification".to_string()
        })?;
    let powershell = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let script = r#"$signature = Get-AuthenticodeSignature -LiteralPath $env:BUZZ_PROVIDER_SIGNATURE_TARGET; if ($signature.Status -ne 'Valid' -or $null -eq $signature.SignerCertificate) { exit 41 }; [Console]::Out.Write($signature.SignerCertificate.Subject)"#;
    let output = std::process::Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("BUZZ_PROVIDER_SIGNATURE_TARGET", binary)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("provider signature verification failed to run: {error}"))?;
    if !output.status.success() {
        return Err("provider has no valid trusted Authenticode signature".to_string());
    }
    let subject = String::from_utf8(output.stdout)
        .map_err(|_| "provider signer subject is not valid UTF-8".to_string())?;
    if !signer_subject_allowed(configured, subject.trim()) {
        return Err("provider Authenticode signer is not approved by this Buzz build".to_string());
    }
    Ok(())
}

/// Non-Windows builds have no platform signature policy.
#[cfg(not(windows))]
pub(super) fn verify_provider_platform_signature(_binary: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(any(windows, test))]
fn signer_subject_allowed(configured: &str, actual: &str) -> bool {
    configured
        .split(';')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .any(|candidate| candidate.eq_ignore_ascii_case(actual))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_platform_provider_filenames() {
        assert_eq!(
            provider_id_from_filename("buzz-backend-kubernetes"),
            Some("kubernetes")
        );
        assert_eq!(
            provider_id_from_filename("buzz-backend-kubernetes.EXE"),
            Some("kubernetes")
        );
        assert_eq!(
            provider_id_from_filename("buzz-backend-claude-code.cmd"),
            Some("claude-code")
        );
        assert_eq!(provider_id_from_filename("buzz-backend-"), None);
        assert_eq!(provider_id_from_filename("other"), None);
    }

    #[test]
    fn builds_the_windows_user_provider_directory() {
        assert_eq!(
            windows_user_provider_dir(Some(OsString::from("C:\\Users\\Ross\\AppData\\Local"))),
            Some(
                PathBuf::from("C:\\Users\\Ross\\AppData\\Local")
                    .join("Buzz")
                    .join("providers")
            )
        );
        assert_eq!(windows_user_provider_dir(None), None);
    }

    #[test]
    fn signer_allowlist_is_trimmed_and_case_insensitive() {
        assert!(signer_subject_allowed(
            "CN=Example Publisher; CN=Other Signer ",
            "cn=example publisher"
        ));
        assert!(!signer_subject_allowed(
            "CN=Example Publisher; CN=Other Signer",
            "CN=Unknown"
        ));
    }
}
