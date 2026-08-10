use serde::Serialize;

use super::*;

const NXTLINQ_GATEWAY_RUNTIME_ID: &str = "nxtlinq-authorization-gateway";
const NXTLINQ_GATEWAY_INSTALL_COMMAND: &str = "npm install -g @nxtlinq/authorization-gateway";

/// Discover the optional Nxtlinq authorization wrapper without treating it as
/// an ACP runtime. Wrapper identity must stay separate from the downstream
/// harness so runtime readiness/model metadata continue to describe the Agent.
#[tauri::command]
pub async fn discover_nxtlinq_authorization_gateway(
) -> Result<crate::managed_agents::CommandAvailabilityInfo, String> {
    tokio::task::spawn_blocking(|| {
        crate::managed_agents::refresh_login_shell_path();
        crate::managed_agents::clear_resolve_cache();
        crate::managed_agents::command_availability(NXTLINQ_GATEWAY_RUNTIME_ID)
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))
}

/// Install the Nxtlinq wrapper into Buzz's private npm prefix. This deliberately
/// installs only the executable: project policy/signing material remains owned
/// by the project authorization owner and deployment operator.
#[tauri::command]
pub async fn install_nxtlinq_authorization_gateway(
    app: tauri::AppHandle,
    force: Option<bool>,
) -> Result<InstallRuntimeResult, String> {
    tokio::task::spawn_blocking(move || {
        install_nxtlinq_gateway_blocking(&app, force.unwrap_or(false))
    })
    .await
    .map_err(|e| format!("install task panicked: {e}"))?
}

fn install_nxtlinq_gateway_blocking(
    app: &tauri::AppHandle,
    force: bool,
) -> Result<InstallRuntimeResult, String> {
    crate::managed_agents::refresh_login_shell_path();
    crate::managed_agents::clear_resolve_cache();

    {
        let mut set = active_installs()
            .lock()
            .map_err(|_| "install lock poisoned".to_string())?;
        if !set.insert(NXTLINQ_GATEWAY_RUNTIME_ID.to_string()) {
            return Err("a Nxtlinq Gateway install is already in progress".to_string());
        }
    }

    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(mut set) = active_installs().lock() {
                set.remove(NXTLINQ_GATEWAY_RUNTIME_ID);
            }
        }
    }
    let _guard = Guard;
    let reporter = InstallReporter::for_run(app, NXTLINQ_GATEWAY_RUNTIME_ID);

    if !force && crate::managed_agents::resolve_command(NXTLINQ_GATEWAY_RUNTIME_ID).is_some() {
        return Ok(InstallRuntimeResult {
            success: true,
            steps: Vec::new(),
            restarted_count: 0,
            failed_restart_count: 0,
            log_path: reporter.log_path(),
        });
    }

    let mut steps = Vec::new();
    let use_managed_npm = managed_node_runtime_supported();
    if use_managed_npm {
        if let Err(step) = ensure_managed_node_runtime_blocking() {
            reporter.record_step(&mut steps, *step);
            return Ok(reporter.failed(steps));
        }
    }

    let planned = if use_managed_npm {
        match managed_npm_command(NXTLINQ_GATEWAY_INSTALL_COMMAND) {
            Ok(Some(command)) => command,
            Ok(None) => NXTLINQ_GATEWAY_INSTALL_COMMAND.to_string(),
            Err(step) => {
                reporter.record_step(&mut steps, *step);
                return Ok(reporter.failed(steps));
            }
        }
    } else {
        NXTLINQ_GATEWAY_INSTALL_COMMAND.to_string()
    };

    let mut result = run_install_command_with_retry("authorization-wrapper", &planned, &reporter);
    if !result.success && result.hint.is_none() {
        result.hint = npm_eacces_hint(&result.stderr, NXTLINQ_GATEWAY_INSTALL_COMMAND);
    }
    let success = result.success;
    steps.push(result);
    if !success {
        return Ok(reporter.failed(steps));
    }

    crate::managed_agents::refresh_login_shell_path();
    crate::managed_agents::clear_resolve_cache();
    if crate::managed_agents::resolve_command(NXTLINQ_GATEWAY_RUNTIME_ID).is_none() {
        reporter.record_step(
            &mut steps,
            crate::managed_agents::InstallStepResult {
                step: "verification".to_string(),
                command: NXTLINQ_GATEWAY_RUNTIME_ID.to_string(),
                success: false,
                stdout: String::new(),
                stderr: "installation completed, but Buzz could not resolve the Gateway executable"
                    .to_string(),
                exit_code: None,
                hint: Some("Restart Buzz, then try enabling Nxtlinq again.".to_string()),
            },
        );
        return Ok(reporter.failed(steps));
    }

    Ok(InstallRuntimeResult {
        success: true,
        steps,
        restarted_count: 0,
        failed_restart_count: 0,
        log_path: reporter.log_path(),
    })
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqSetupCheckItem {
    pub(super) id: &'static str,
    label: &'static str,
    pub(super) status: &'static str,
    path: Option<String>,
    detail: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NxtlinqSetupCheckResult {
    pub(super) ready: bool,
    pub(super) checks: Vec<NxtlinqSetupCheckItem>,
    signer_key_id: Option<String>,
    pub(super) error: Option<String>,
}

fn setup_item(
    id: &'static str,
    label: &'static str,
    status: &'static str,
    path: Option<&std::path::Path>,
    detail: Option<String>,
) -> NxtlinqSetupCheckItem {
    NxtlinqSetupCheckItem {
        id,
        label,
        status,
        path: path.map(|value| value.display().to_string()),
        detail,
    }
}

/// Verify the Nxtlinq project binding before saving an Agent wrapper. Presence
/// and operator-path checks happen in the Host; cryptographic verification is
/// delegated to the installed Gateway's `--check` mode so Buzz never
/// reimplements Nxtlinq trust semantics or handles signing keys.
#[tauri::command]
pub async fn check_nxtlinq_authorization_setup(
    project_root: String,
    trust_store: String,
    receipt_directory: String,
) -> Result<NxtlinqSetupCheckResult, String> {
    tokio::task::spawn_blocking(move || {
        check_nxtlinq_authorization_setup_blocking(&project_root, &trust_store, &receipt_directory)
    })
    .await
    .map_err(|error| format!("setup check task panicked: {error}"))
}

pub(super) fn check_nxtlinq_authorization_setup_blocking(
    project_root: &str,
    trust_store: &str,
    receipt_directory: &str,
) -> NxtlinqSetupCheckResult {
    use std::path::PathBuf;

    let project = PathBuf::from(project_root);
    let trust = PathBuf::from(trust_store);
    let receipts = PathBuf::from(receipt_directory);
    let manifest = project.join("nxtlinq").join("agent.manifest.json");
    let signature = project.join("nxtlinq").join("agent.manifest.sig");
    let mut checks = Vec::new();

    let project_ready = project.is_absolute() && project.is_dir();
    checks.push(setup_item(
        "project",
        "Agent workspace",
        if project_ready { "ready" } else { "invalid" },
        Some(&project),
        (!project_ready).then(|| "Select an existing absolute project directory.".to_string()),
    ));
    let manifest_ready = project_ready && manifest.is_file();
    checks.push(setup_item(
        "manifest",
        "Signed manifest",
        if manifest_ready { "found" } else { "missing" },
        Some(&manifest),
        (!manifest_ready).then(|| "Create and review nxtlinq/agent.manifest.json.".to_string()),
    ));
    let signature_ready = project_ready && signature.is_file();
    checks.push(setup_item(
        "signature",
        "Manifest signature",
        if signature_ready { "found" } else { "missing" },
        Some(&signature),
        (!signature_ready)
            .then(|| "Sign the manifest with the project owner's private key.".to_string()),
    ));

    let project_canonical = std::fs::canonicalize(&project).ok();
    let trust_canonical = std::fs::canonicalize(&trust).ok();
    let trust_outside = match (&project_canonical, &trust_canonical) {
        (Some(root), Some(path)) => !path.starts_with(root),
        _ => !trust.starts_with(&project),
    };
    let trust_ready = trust.is_absolute() && trust.is_file() && trust_outside;
    checks.push(setup_item(
        "trustStore",
        "Trust store",
        if trust_ready {
            "found"
        } else if trust.is_file() {
            "invalid"
        } else {
            "missing"
        },
        Some(&trust),
        (!trust_ready).then(|| {
            if trust.is_file() && !trust_outside {
                "Move the trust store outside the Agent-writable project.".to_string()
            } else {
                "Provide an external trusted-signers.json created by the deployment operator."
                    .to_string()
            }
        }),
    ));

    let receipts_outside = project_canonical
        .as_ref()
        .is_none_or(|root| !receipts.starts_with(root));
    let receipt_error = if receipts.is_absolute() && receipts_outside {
        crate::commands::nxtlinq_authorization::prepare_receipt_directory(&receipts).err()
    } else {
        Some("Use an operator-controlled directory outside the Agent workspace.".to_string())
    };
    let receipt_valid = receipt_error.is_none();
    checks.push(setup_item(
        "receipts",
        "Receipt directory",
        if receipt_valid { "ready" } else { "invalid" },
        Some(&receipts),
        receipt_error,
    ));

    if !(project_ready && manifest_ready && signature_ready && trust_ready && receipt_valid) {
        checks.push(setup_item(
            "trustedSigner",
            "Trusted signer",
            "blocked",
            None,
            Some("Complete the missing setup before signature verification.".to_string()),
        ));
        return NxtlinqSetupCheckResult {
            ready: false,
            checks,
            signer_key_id: None,
            error: Some("Nxtlinq setup is incomplete.".to_string()),
        };
    }

    crate::managed_agents::refresh_login_shell_path();
    crate::managed_agents::clear_resolve_cache();
    let Some(gateway) = crate::managed_agents::resolve_command(NXTLINQ_GATEWAY_RUNTIME_ID) else {
        checks.push(setup_item(
            "trustedSigner",
            "Trusted signer",
            "blocked",
            None,
            Some("Install Nxtlinq Gateway before verifying the signed policy.".to_string()),
        ));
        return NxtlinqSetupCheckResult {
            ready: false,
            checks,
            signer_key_id: None,
            error: Some("Nxtlinq Gateway is not installed.".to_string()),
        };
    };

    let mut path_parts = Vec::new();
    if let Some(path) = crate::managed_agents::buzz_managed_node_bin_dir() {
        path_parts.push(path);
    }
    if let Some(path) = crate::managed_agents::buzz_managed_npm_bin_dir() {
        path_parts.push(path);
    }
    if let Some(path) = std::env::var_os("PATH") {
        path_parts.extend(std::env::split_paths(&path));
    }
    let mut command = std::process::Command::new(gateway);
    command.args([
        "--check",
        "--project",
        project_root,
        "--trust-store",
        trust_store,
        "--receipt-dir",
        receipt_directory,
    ]);
    if let Ok(path) = std::env::join_paths(path_parts) {
        command.env("PATH", path);
    }
    let output = command.output();
    let report = match output {
        Ok(output) if output.status.success() => {
            serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()
        }
        Ok(output) => {
            let detail = String::from_utf8_lossy(&output.stderr)
                .trim()
                .trim_start_matches("Error: ")
                .to_string();
            checks.push(setup_item(
                "trustedSigner",
                "Trusted signer",
                "invalid",
                None,
                Some(if detail.is_empty() {
                    "Gateway rejected the signed manifest or trust store.".to_string()
                } else {
                    detail.clone()
                }),
            ));
            return NxtlinqSetupCheckResult {
                ready: false,
                checks,
                signer_key_id: None,
                error: Some(if detail.is_empty() {
                    "Nxtlinq cryptographic verification failed.".to_string()
                } else {
                    detail
                }),
            };
        }
        Err(error) => {
            checks.push(setup_item(
                "trustedSigner",
                "Trusted signer",
                "invalid",
                None,
                Some(format!("Could not run Gateway verification: {error}")),
            ));
            return NxtlinqSetupCheckResult {
                ready: false,
                checks,
                signer_key_id: None,
                error: Some("Could not run Nxtlinq Gateway verification.".to_string()),
            };
        }
    };
    let signer_key_id = report
        .as_ref()
        .and_then(|value| value.get("signerKeyId"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let verified = report
        .as_ref()
        .and_then(|value| value.get("ready"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && signer_key_id.is_some();
    checks.push(setup_item(
        "trustedSigner",
        "Trusted signer",
        if verified { "valid" } else { "invalid" },
        None,
        signer_key_id
            .as_ref()
            .map(|key_id| format!("Signature verified for {key_id}."))
            .or_else(|| Some("Gateway returned an invalid verification report.".to_string())),
    ));
    NxtlinqSetupCheckResult {
        ready: verified,
        checks,
        signer_key_id,
        error: (!verified).then(|| "Nxtlinq cryptographic verification failed.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_check_reports_missing_operator_material_without_spawning_gateway() {
        let root = tempfile::tempdir().expect("temp project");
        let project = root.path().join("project");
        std::fs::create_dir(&project).expect("project directory");
        let receipts = root.path().join("operator/receipts");
        let report = check_nxtlinq_authorization_setup_blocking(
            &project.display().to_string(),
            &root
                .path()
                .join("operator/trusted-signers.json")
                .display()
                .to_string(),
            &receipts.display().to_string(),
        );

        assert!(!report.ready);
        assert_eq!(
            report.error.as_deref(),
            Some("Nxtlinq setup is incomplete.")
        );
        for (id, status) in [
            ("manifest", "missing"),
            ("signature", "missing"),
            ("trustStore", "missing"),
            ("trustedSigner", "blocked"),
        ] {
            assert!(report
                .checks
                .iter()
                .any(|check| check.id == id && check.status == status));
        }
        assert!(receipts.is_dir());
    }
}
