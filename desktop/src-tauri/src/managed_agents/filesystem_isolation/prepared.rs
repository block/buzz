use super::*;

/// Create one fresh owner-inspectable root without starting any harness.
/// A second prepare for the same identity fails until the first is consumed
/// or explicitly aborted.
pub fn prepare_isolated_agent_run(
    profile: &FilesystemIsolationProfile,
    identity_pubkey: &str,
    desktop_instance_id: &str,
    acp_command: &Path,
) -> Result<PreparedFilesystemIsolation, String> {
    let key = identity_pubkey.to_ascii_lowercase();
    validate_identity(&key)?;
    recover_abandoned_isolation_runs()?;
    ensure_no_existing_isolation_receipt(&key)?;

    let mut registry = prepared_isolation_registry()
        .lock()
        .map_err(|error| format!("prepared isolation registry lock poisoned: {error}"))?;
    if registry.contains_key(&key) {
        return Err(format!(
            "agent {key} already has a prepared filesystem-isolation run"
        ));
    }
    let run = create_isolation_run(profile, &key, desktop_instance_id, acp_command)?;
    let prepared = PreparedFilesystemIsolation {
        identity_pubkey: key.clone(),
        run_id: run.attestation.run_id.clone(),
        run_root: run.attestation.run_root.clone(),
        attestation: run.attestation.clone(),
    };
    registry.insert(
        key,
        PreparedIsolationRun {
            run,
            profile: profile.clone(),
            desktop_instance_id: desktop_instance_id.to_string(),
            acp_command: acp_command.to_path_buf(),
        },
    );
    Ok(prepared)
}

/// Consume exactly one prepared root. Removal from the registry happens
/// before the durable `spawning` transition, so no concurrent start can reuse
/// the same root.
pub fn consume_prepared_isolated_agent_command(
    profile: &FilesystemIsolationProfile,
    identity_pubkey: &str,
    desktop_instance_id: &str,
    acp_command: &Path,
) -> Result<(Command, FilesystemIsolationRun), String> {
    let key = identity_pubkey.to_ascii_lowercase();
    let prepared = prepared_isolation_registry()
        .lock()
        .map_err(|error| format!("prepared isolation registry lock poisoned: {error}"))?
        .remove(&key)
        .ok_or_else(|| {
            format!("agent {key} requires an owner-prepared filesystem-isolation run before start")
        })?;
    if prepared.profile != *profile
        || prepared.desktop_instance_id != desktop_instance_id
        || prepared.acp_command != acp_command
    {
        return Err(
            "prepared filesystem-isolation run no longer matches the effective agent configuration"
                .to_string(),
        );
    }
    let mut run = prepared.run;
    let command = command_for_prepared_run(&mut run, acp_command)?;
    Ok((command, run))
}

pub fn abort_prepared_isolated_agent_run(
    identity_pubkey: &str,
    expected_run_id: &str,
) -> Result<(), String> {
    let key = identity_pubkey.to_ascii_lowercase();
    let mut registry = prepared_isolation_registry()
        .lock()
        .map_err(|error| format!("prepared isolation registry lock poisoned: {error}"))?;
    let current = registry
        .get(&key)
        .ok_or_else(|| format!("agent {key} has no prepared filesystem-isolation run"))?;
    if current.run.attestation.run_id != expected_run_id {
        return Err("prepared filesystem-isolation run id does not match".to_string());
    }
    registry.remove(&key);
    Ok(())
}

pub fn get_prepared_isolated_agent_run(
    identity_pubkey: &str,
) -> Result<Option<PreparedFilesystemIsolation>, String> {
    let key = identity_pubkey.to_ascii_lowercase();
    validate_identity(&key)?;
    let registry = prepared_isolation_registry()
        .lock()
        .map_err(|error| format!("prepared isolation registry lock poisoned: {error}"))?;
    Ok(registry
        .get(&key)
        .map(|prepared| PreparedFilesystemIsolation {
            identity_pubkey: key,
            run_id: prepared.run.attestation.run_id.clone(),
            run_root: prepared.run.attestation.run_root.clone(),
            attestation: prepared.run.attestation.clone(),
        }))
}

pub(super) fn create_isolation_run(
    profile: &FilesystemIsolationProfile,
    identity_pubkey: &str,
    desktop_instance_id: &str,
    acp_command: &Path,
) -> Result<FilesystemIsolationRun, String> {
    let FilesystemIsolationProfile::Ephemeral { read_only_roots } = profile;
    validate_identity(identity_pubkey)?;

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (read_only_roots, desktop_instance_id, acp_command);
        return Err(
            "ephemeral filesystem isolation is currently supported only on macOS".to_string(),
        );
    }

    #[cfg(target_os = "macos")]
    {
        recover_abandoned_isolation_runs()?;
        let sandbox_exec = Path::new("/usr/bin/sandbox-exec");
        if !sandbox_exec.is_file() {
            return Err("macOS filesystem isolation requires /usr/bin/sandbox-exec".to_string());
        }
        let (base, run_id, root) = create_run_root(identity_pubkey)?;
        let result = (|| {
            let home = root.join("home");
            let temp = root.join("tmp");
            create_private_dir(&home)?;
            create_private_dir(&temp)?;
            let denied_roots = denied_roots()?;
            let mut allowed_read_roots = system_read_roots();
            allowed_read_roots.extend(validate_read_only_roots(
                read_only_roots,
                &protected_data_roots()?,
            )?);
            allowed_read_roots.extend(executable_read_roots(acp_command)?);
            allowed_read_roots.push(root.clone());
            normalize_paths(&mut allowed_read_roots);
            let attestation = FilesystemIsolationAttestation {
                version: 1,
                enforcement: "macos_seatbelt_process_tree_control_plane_v1",
                identity_pubkey: identity_pubkey.to_ascii_lowercase(),
                run_id: run_id.clone(),
                run_root: root.clone(),
                allowed_read_roots,
                allowed_write_roots: vec![root.clone()],
                denied_roots,
            };
            // Validate the profile before publishing the durable prepared receipt.
            let _ = seatbelt_profile(&attestation)?;
            let control = IsolationControlPlane::start(&attestation)?;
            let ownership_path = receipts_dir(&base)?.join(format!("{run_id}.json"));
            let ownership = IsolationRunOwnership {
                version: 1,
                identity_pubkey: identity_pubkey.to_ascii_lowercase(),
                desktop_instance_id: desktop_instance_id.to_string(),
                run_id,
                run_root: root.clone(),
                desktop_pid: std::process::id(),
                agent_pid: None,
                phase: Some(IsolationRunPhase::Prepared),
            };
            write_ownership_receipt(&ownership_path, &ownership, true)?;
            Ok(FilesystemIsolationRun {
                root: root.clone(),
                base: base.clone(),
                ownership_path,
                ownership,
                control,
                attestation,
            })
        })();
        if result.is_err() {
            let _ = remove_run_root(&base, &root);
        }
        result
    }
}

#[cfg(target_os = "macos")]
pub(super) fn command_for_prepared_run(
    run: &mut FilesystemIsolationRun,
    acp_command: &Path,
) -> Result<Command, String> {
    run.ownership.phase = Some(IsolationRunPhase::Spawning);
    write_ownership_receipt(&run.ownership_path, &run.ownership, false)?;
    let home = run.root.join("home");
    let temp = run.root.join("tmp");
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .arg("-p")
        .arg(seatbelt_profile(&run.attestation)?)
        .arg(acp_command)
        .current_dir(&run.root)
        .env("HOME", &home)
        .env("TMPDIR", &temp)
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env(ISOLATION_RUN_ROOT_ENV, &run.root)
        .env(
            ISOLATION_ATTESTATION_ENV,
            serde_json::to_string(&run.attestation)
                .map_err(|error| format!("failed to serialize isolation receipt: {error}"))?,
        );
    Ok(command)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn command_for_prepared_run(
    _run: &mut FilesystemIsolationRun,
    _acp_command: &Path,
) -> Result<Command, String> {
    Err("ephemeral filesystem isolation is currently supported only on macOS".to_string())
}
