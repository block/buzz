use std::io::{Read, Write};

const TRUSTED_BUZZ_ACP_SHA256: &str =
    "ab4b527eb63b701e6ee931358b5e14914c3d0826da1e75dea6cfdf6cf72feab7";
const TRUSTED_BUZZ_ACP: &str = "/Users/gabriel/.buzz/RUNTIME/buzz-acp/ab4b527eb63b701e6ee931358b5e14914c3d0826da1e75dea6cfdf6cf72feab7/buzz-acp";
const CONTEXT_ENGINE_INSTALL_LOCK: &str =
    "/Users/gabriel/.buzz/RUNTIME/deployment-control/install.lock";
const TRUSTED_CONTEXT_ENGINE_NODE: &str = "/Users/gabriel/.buzz/RUNTIME/trusted-node/d36b3d980963d44bd2c5e844fac4cfeee26a167b744287a4e74a9575af9d0559/node";
const TRUSTED_GABE_CONTEXT_ENGINE_ADAPTER: &str = "/Users/gabriel/.buzz/RUNTIME/context-engine/96a8efaf20cbc1cb92fb2ae2eca5a0bdefabba42f9cd6e2ca21299c724bd7c5c/scripts/gabe-acp.mjs";
const TRUSTED_STACY_CONTEXT_ENGINE_ADAPTER: &str = "/Users/gabriel/.buzz/RUNTIME/stacy-context-engine/96a8efaf20cbc1cb92fb2ae2eca5a0bdefabba42f9cd6e2ca21299c724bd7c5c/scripts/gabe-acp.mjs";
const CONTEXT_ENGINE_IDENTITY_KEYS: [&str; 4] = [
    "BUZZ_GABE_AGENT_PUBKEY",
    "BUZZ_GABE_OWNER_PUBKEY",
    "BUZZ_STACY_AGENT_PUBKEY",
    "BUZZ_STACY_OWNER_PUBKEY",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TrustedContextEngineHarness {
    Gabe,
    Stacy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ContextEngineHarnessClass {
    Exact(TrustedContextEngineHarness),
    ReservedInvalid,
    Ordinary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct TrustedBuzzAcpIdentity {
    length: u64,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

fn uses_reserved_runtime_path(value: &str) -> bool {
    fn has_reserved_prefix(value: &str) -> bool {
        let value = value.to_ascii_lowercase();
        value == "/users/gabriel/.buzz/runtime"
            || value.starts_with("/users/gabriel/.buzz/runtime/")
    }

    has_reserved_prefix(value)
        || std::path::Path::new(value)
            .canonicalize()
            .ok()
            .is_some_and(|path| has_reserved_prefix(&path.to_string_lossy()))
}

pub(super) fn classify_context_engine_harness(
    command: &str,
    args: &[String],
) -> ContextEngineHarnessClass {
    if command == TRUSTED_CONTEXT_ENGINE_NODE && args.len() == 1 {
        return match args[0].as_str() {
            TRUSTED_GABE_CONTEXT_ENGINE_ADAPTER => {
                ContextEngineHarnessClass::Exact(TrustedContextEngineHarness::Gabe)
            }
            TRUSTED_STACY_CONTEXT_ENGINE_ADAPTER => {
                ContextEngineHarnessClass::Exact(TrustedContextEngineHarness::Stacy)
            }
            _ => ContextEngineHarnessClass::ReservedInvalid,
        };
    }
    if uses_reserved_runtime_path(command)
        || args.iter().any(|value| uses_reserved_runtime_path(value))
    {
        ContextEngineHarnessClass::ReservedInvalid
    } else {
        ContextEngineHarnessClass::Ordinary
    }
}

pub(super) fn resolve_outer_acp_command(
    class: ContextEngineHarnessClass,
    requested_command: &str,
    resolve_requested: impl FnOnce(&str) -> Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    match class {
        ContextEngineHarnessClass::Exact(_) => Some(std::path::PathBuf::from(TRUSTED_BUZZ_ACP)),
        ContextEngineHarnessClass::ReservedInvalid | ContextEngineHarnessClass::Ordinary => {
            resolve_requested(requested_command)
        }
    }
}

pub(super) fn prepare_context_engine_launch(
    resolved_agent_command: &str,
    agent_args: &[String],
    requested_acp_command: &str,
    resolve_requested: impl FnOnce(&str) -> Option<std::path::PathBuf>,
) -> Result<
    (
        Option<TrustedContextEngineHarness>,
        std::path::PathBuf,
        Option<TrustedBuzzAcpIdentity>,
    ),
    String,
> {
    let class = classify_context_engine_harness(resolved_agent_command, agent_args);
    let harness = match class {
        ContextEngineHarnessClass::Exact(harness) => Some(harness),
        ContextEngineHarnessClass::ReservedInvalid => {
            return Err(
                "reserved Context Engine runtime path does not match the reviewed harness tuple"
                    .to_string(),
            );
        }
        ContextEngineHarnessClass::Ordinary => None,
    };
    let outer_acp = resolve_outer_acp_command(class, requested_acp_command, resolve_requested)
        .ok_or_else(|| {
            crate::managed_agents::missing_command_message(
                requested_acp_command,
                "ACP harness command",
            )
        })?;
    let identity = harness
        .map(|_| validate_trusted_buzz_acp(&outer_acp))
        .transpose()?;
    Ok((harness, outer_acp, identity))
}

fn sha256_reader(reader: &mut std::fs::File) -> Result<String, String> {
    use sha2::{Digest as _, Sha256};

    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| format!("cannot hash trusted buzz-acp binary: {error}"))?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(hex::encode(digest.finalize()))
}

#[cfg(target_os = "macos")]
fn current_user_uid() -> Result<u32, String> {
    let output = std::process::Command::new("/usr/bin/id")
        .arg("-u")
        .env_clear()
        .output()
        .map_err(|error| format!("cannot determine current uid: {error}"))?;
    if !output.status.success() {
        return Err("cannot determine current uid".to_string());
    }
    String::from_utf8(output.stdout)
        .map_err(|_| "current uid was not UTF-8".to_string())?
        .trim()
        .parse()
        .map_err(|_| "current uid was not numeric".to_string())
}

#[cfg(target_os = "macos")]
fn validate_frozen_directory(path: &std::path::Path, current_uid: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("trusted buzz-acp directory is unavailable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("trusted buzz-acp directory must be a non-symlink directory".to_string());
    }
    if path
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize trusted buzz-acp directory: {error}"))?
        != path
    {
        return Err("trusted buzz-acp directory path must be canonical".to_string());
    }
    let owner = std::os::unix::fs::MetadataExt::uid(&metadata);
    let mode = metadata.permissions().mode() & 0o7777;
    let flags = std::os::macos::fs::MetadataExt::st_flags(&metadata);
    if owner != current_uid || mode != 0o555 || flags & libc::UF_IMMUTABLE == 0 {
        return Err(
            "trusted buzz-acp directories must be current-user owned, mode 0555, and immutable"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_trusted_buzz_acp_at(
    resolved_command: &std::path::Path,
    expected_path: &std::path::Path,
    expected_sha256: &str,
    require_immutable: bool,
) -> Result<TrustedBuzzAcpIdentity, String> {
    let path_metadata = std::fs::symlink_metadata(expected_path)
        .map_err(|error| format!("trusted buzz-acp binary is unavailable: {error}"))?;
    if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
        return Err("trusted buzz-acp path must be a regular non-symlink file".to_string());
    }
    let canonical = expected_path
        .canonicalize()
        .map_err(|error| format!("cannot canonicalize trusted buzz-acp binary: {error}"))?;
    if canonical != expected_path || resolved_command != canonical {
        return Err(format!(
            "trusted Context Engine requires exact outer harness {}",
            expected_path.display()
        ));
    }

    #[cfg(target_os = "macos")]
    let current_uid = current_user_uid()?;
    #[cfg(target_os = "macos")]
    if require_immutable {
        let version_directory = expected_path
            .parent()
            .ok_or_else(|| "trusted buzz-acp binary has no version directory".to_string())?;
        let namespace_directory = version_directory
            .parent()
            .ok_or_else(|| "trusted buzz-acp binary has no namespace directory".to_string())?;
        validate_frozen_directory(namespace_directory, current_uid)?;
        validate_frozen_directory(version_directory, current_uid)?;
    }

    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(expected_path)
        .map_err(|error| format!("cannot open trusted buzz-acp binary: {error}"))?;
    let before = file
        .metadata()
        .map_err(|error| format!("cannot inspect trusted buzz-acp binary: {error}"))?;
    if !before.is_file() || sha256_reader(&mut file)? != expected_sha256 {
        return Err("trusted buzz-acp binary digest does not match the reviewed build".to_string());
    }
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot re-inspect trusted buzz-acp binary: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if before.dev() != metadata.dev()
            || before.ino() != metadata.ino()
            || before.len() != metadata.len()
        {
            return Err("trusted buzz-acp binary changed while it was validated".to_string());
        }
    }

    #[cfg(target_os = "macos")]
    if require_immutable {
        use std::os::unix::fs::PermissionsExt as _;

        let owner = std::os::unix::fs::MetadataExt::uid(&metadata);
        let mode = metadata.permissions().mode() & 0o7777;
        let flags = std::os::macos::fs::MetadataExt::st_flags(&metadata);
        if owner != current_uid || mode != 0o555 || flags & libc::UF_IMMUTABLE == 0 {
            return Err(
                "trusted buzz-acp binary must be current-user owned, mode 0555, and immutable"
                    .to_string(),
            );
        }
    }
    let _ = require_immutable;
    Ok(TrustedBuzzAcpIdentity {
        length: metadata.len(),
        #[cfg(unix)]
        device: std::os::unix::fs::MetadataExt::dev(&metadata),
        #[cfg(unix)]
        inode: std::os::unix::fs::MetadataExt::ino(&metadata),
    })
}

fn ensure_context_engine_install_idle_at(lock_path: &std::path::Path) -> Result<(), String> {
    match std::fs::symlink_metadata(lock_path) {
        Ok(_) => Err(
            "Context Engine runtime installation is in progress; retry after it completes"
                .to_string(),
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "cannot verify Context Engine deployment lock state: {error}"
        )),
    }
}

pub(super) fn validate_trusted_buzz_acp(
    resolved_command: &std::path::Path,
) -> Result<TrustedBuzzAcpIdentity, String> {
    ensure_context_engine_install_idle_at(std::path::Path::new(CONTEXT_ENGINE_INSTALL_LOCK))?;
    validate_trusted_buzz_acp_at(
        resolved_command,
        std::path::Path::new(TRUSTED_BUZZ_ACP),
        TRUSTED_BUZZ_ACP_SHA256,
        true,
    )
}

pub(super) fn revalidate_trusted_buzz_acp(
    resolved_command: &std::path::Path,
    expected_identity: TrustedBuzzAcpIdentity,
) -> Result<(), String> {
    let current_identity = validate_trusted_buzz_acp(resolved_command)?;
    if current_identity != expected_identity {
        return Err("trusted buzz-acp binary identity changed before spawn".to_string());
    }
    Ok(())
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn apply_context_engine_identity(
    command: &mut std::process::Command,
    harness: Option<TrustedContextEngineHarness>,
    agent_pubkey: &str,
    owner_pubkey: Option<&str>,
) -> Result<(), String> {
    for key in CONTEXT_ENGINE_IDENTITY_KEYS {
        command.env_remove(key);
    }
    let Some(harness) = harness else {
        return Ok(());
    };
    let owner_pubkey = owner_pubkey
        .filter(|value| is_lower_hex_64(value))
        .ok_or_else(|| {
            "trusted Context Engine requires an authenticated owner pubkey".to_string()
        })?;
    if !is_lower_hex_64(agent_pubkey) {
        return Err("trusted Context Engine requires a canonical managed-agent pubkey".to_string());
    }
    let (agent_key, owner_key) = match harness {
        TrustedContextEngineHarness::Gabe => ("BUZZ_GABE_AGENT_PUBKEY", "BUZZ_GABE_OWNER_PUBKEY"),
        TrustedContextEngineHarness::Stacy => {
            ("BUZZ_STACY_AGENT_PUBKEY", "BUZZ_STACY_OWNER_PUBKEY")
        }
    };
    command
        .env(agent_key, agent_pubkey)
        .env(owner_key, owner_pubkey);
    Ok(())
}

pub(super) fn install_acp_credential_stdin(
    command: &mut std::process::Command,
    private_key: &str,
    auth_tag: Option<&str>,
) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    #[derive(serde::Serialize)]
    struct CredentialEnvelope<'a> {
        private_key: &'a str,
        auth_tag: Option<&'a str>,
    }

    let payload = serde_json::to_vec(&CredentialEnvelope {
        private_key,
        auth_tag,
    })
    .map(zeroize::Zeroizing::new)
    .map_err(|error| format!("failed to encode ACP credential envelope: {error}"))?;
    command
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("NOSTR_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .env("BUZZ_ACP_CREDENTIAL_STDIN", "true")
        .stdin(std::process::Stdio::piped());
    Ok(payload)
}

pub(super) fn deliver_acp_credentials(
    child: &mut std::process::Child,
    payload: &[u8],
) -> std::io::Result<()> {
    let mut stdin = child.stdin.take().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "ACP credential stdin was not piped",
        )
    })?;
    stdin.write_all(payload)
}

pub(super) fn child_rust_log_filter() -> String {
    match std::env::var("RUST_LOG") {
        Ok(existing) if existing.contains("buzz_acp") => existing,
        Ok(existing) if !existing.trim().is_empty() => format!("{existing},buzz_acp=info"),
        _ => "buzz_acp=info".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_harnesses_require_exact_paths_and_fail_closed_on_reserved_drift() {
        assert_eq!(
            classify_context_engine_harness(
                TRUSTED_CONTEXT_ENGINE_NODE,
                &[TRUSTED_GABE_CONTEXT_ENGINE_ADAPTER.to_string()]
            ),
            ContextEngineHarnessClass::Exact(TrustedContextEngineHarness::Gabe)
        );
        assert_eq!(
            classify_context_engine_harness(
                TRUSTED_CONTEXT_ENGINE_NODE,
                &[TRUSTED_STACY_CONTEXT_ENGINE_ADAPTER.to_string()]
            ),
            ContextEngineHarnessClass::Exact(TrustedContextEngineHarness::Stacy)
        );
        assert_eq!(
            classify_context_engine_harness(
                "node",
                &[TRUSTED_GABE_CONTEXT_ENGINE_ADAPTER.to_string()]
            ),
            ContextEngineHarnessClass::ReservedInvalid
        );
        assert_eq!(
            classify_context_engine_harness(
                TRUSTED_CONTEXT_ENGINE_NODE,
                &["/tmp/gabe-acp.mjs".to_string()]
            ),
            ContextEngineHarnessClass::ReservedInvalid
        );
        assert_eq!(
            classify_context_engine_harness(TRUSTED_CONTEXT_ENGINE_NODE, &[]),
            ContextEngineHarnessClass::ReservedInvalid
        );
        assert_eq!(
            classify_context_engine_harness(
                "node",
                &[TRUSTED_STACY_CONTEXT_ENGINE_ADAPTER
                    .replace("/.buzz/RUNTIME/", "/.buzz/runtime/")]
            ),
            ContextEngineHarnessClass::ReservedInvalid
        );
        assert_eq!(
            classify_context_engine_harness("/usr/bin/python3", &["agent.py".to_string()]),
            ContextEngineHarnessClass::Ordinary
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rejects_an_unreviewed_outer_binary_before_execution() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = std::env::temp_dir().join(format!(
            "buzz-trusted-outer-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&directory).expect("create outer harness fixture");
        let directory = directory.canonicalize().expect("canonical fixture path");
        let sentinel = directory.join("executed");
        let executable = directory.join("buzz-acp");
        std::fs::write(
            &executable,
            format!("#!/bin/sh\n/usr/bin/touch {}\n", sentinel.display()),
        )
        .expect("write fake outer harness");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755))
            .expect("make fake harness executable");
        let mut fixture = std::fs::File::open(&executable).expect("open fake harness");
        let digest = sha256_reader(&mut fixture).expect("hash fake harness");

        validate_trusted_buzz_acp_at(&executable, &executable, &digest, false)
            .expect("matching fixture is structurally valid");
        assert!(
            validate_trusted_buzz_acp_at(&executable, &executable, &"00".repeat(32), false)
                .is_err()
        );
        assert!(validate_trusted_buzz_acp_at(
            std::path::Path::new("/usr/bin/true"),
            &executable,
            &digest,
            false,
        )
        .is_err());
        assert!(
            !sentinel.exists(),
            "validation must not execute the candidate"
        );
        std::fs::remove_dir_all(directory).expect("remove outer harness fixture");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn frozen_outer_identity_rejects_special_modes_and_ancestor_swaps() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = std::env::temp_dir().join(format!(
            "buzz-trusted-ancestor-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&root).expect("create ancestor-swap root");
        let root = root
            .canonicalize()
            .expect("canonicalize ancestor-swap root");
        let namespace = root.join("buzz-acp");
        let version = namespace.join("reviewed");
        let executable = version.join("buzz-acp");
        std::fs::create_dir_all(&version).expect("create frozen hierarchy fixture");
        std::fs::copy("/usr/bin/true", &executable).expect("copy frozen executable fixture");
        let mut fixture = std::fs::File::open(&executable).expect("open frozen fixture");
        let digest = sha256_reader(&mut fixture).expect("hash frozen fixture");

        std::fs::set_permissions(&namespace, std::fs::Permissions::from_mode(0o555))
            .expect("set namespace mode");
        std::fs::set_permissions(&version, std::fs::Permissions::from_mode(0o555))
            .expect("set version mode");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o4555))
            .expect("set unsafe special mode");
        let status = std::process::Command::new("/usr/bin/chflags")
            .args(["-R", "uchg"])
            .arg(&namespace)
            .status()
            .expect("freeze unsafe-mode fixture");
        assert!(status.success());
        assert!(validate_trusted_buzz_acp_at(&executable, &executable, &digest, true).is_err());

        std::process::Command::new("/usr/bin/chflags")
            .args(["-R", "nouchg"])
            .arg(&namespace)
            .status()
            .expect("unfreeze fixture before mode repair");
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o555))
            .expect("repair executable mode");
        std::process::Command::new("/usr/bin/chflags")
            .args(["-R", "uchg"])
            .arg(&namespace)
            .status()
            .expect("refreeze reviewed fixture");
        let identity = validate_trusted_buzz_acp_at(&executable, &executable, &digest, true)
            .expect("frozen reviewed fixture is valid");

        std::process::Command::new("/usr/bin/chflags")
            .args(["-R", "nouchg"])
            .arg(&namespace)
            .status()
            .expect("unfreeze namespace for swap probe");
        let displaced = root.join("displaced");
        std::fs::rename(&namespace, &displaced).expect("displace reviewed namespace");
        std::fs::create_dir_all(&version).expect("create replacement hierarchy");
        std::fs::copy("/usr/bin/true", &executable).expect("copy replacement executable");
        for path in [&namespace, &version, &executable] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o555))
                .expect("set replacement mode");
        }
        std::process::Command::new("/usr/bin/chflags")
            .args(["-R", "uchg"])
            .arg(&namespace)
            .status()
            .expect("freeze replacement namespace");
        let replacement = validate_trusted_buzz_acp_at(&executable, &executable, &digest, true)
            .expect("same-content replacement is structurally valid");
        assert_ne!(replacement, identity);

        for path in [&namespace, &displaced] {
            let _ = std::process::Command::new("/usr/bin/chflags")
                .args(["-R", "nouchg"])
                .arg(path)
                .status();
            let _ = std::process::Command::new("/bin/chmod")
                .args(["-R", "u+w"])
                .arg(path)
                .status();
        }
        std::fs::remove_dir_all(root).expect("remove ancestor-swap fixture");
    }

    #[test]
    fn install_lock_blocks_new_launches() {
        let directory = std::env::temp_dir().join(format!(
            "buzz-install-lock-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&directory).expect("create lock fixture");
        let lock = directory.join("install.lock");
        ensure_context_engine_install_idle_at(&lock).expect("absent lock is idle");
        std::fs::write(&lock, b"installing").expect("create install lock");
        assert!(ensure_context_engine_install_idle_at(&lock).is_err());
        std::fs::remove_dir_all(directory).expect("remove lock fixture");
    }

    #[test]
    fn identity_is_derived_and_cross_harness_safe() {
        let agent = "ab".repeat(32);
        let owner = "cd".repeat(32);
        let mut command = std::process::Command::new("/usr/bin/true");
        for key in CONTEXT_ENGINE_IDENTITY_KEYS {
            command.env(key, "imposter");
        }
        apply_context_engine_identity(
            &mut command,
            Some(TrustedContextEngineHarness::Gabe),
            &agent,
            Some(&owner),
        )
        .expect("inject Gabe pins");
        let env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            env["BUZZ_GABE_AGENT_PUBKEY"].as_deref(),
            Some(agent.as_str())
        );
        assert_eq!(
            env["BUZZ_GABE_OWNER_PUBKEY"].as_deref(),
            Some(owner.as_str())
        );
        assert_eq!(env["BUZZ_STACY_AGENT_PUBKEY"], None);
        assert_eq!(env["BUZZ_STACY_OWNER_PUBKEY"], None);
        assert!(apply_context_engine_identity(
            &mut command,
            Some(TrustedContextEngineHarness::Stacy),
            &agent.to_uppercase(),
            Some(&owner),
        )
        .is_err());
    }

    #[test]
    fn credential_stdin_removes_launch_environment_secret_carriers() {
        let mut command = std::process::Command::new("/usr/bin/true");
        command
            .env("BUZZ_PRIVATE_KEY", "private-canary")
            .env("NOSTR_PRIVATE_KEY", "nostr-canary")
            .env("BUZZ_AUTH_TAG", "auth-canary");
        let payload =
            install_acp_credential_stdin(&mut command, "private-canary", Some("auth-canary"))
                .expect("install protected credential stdin");
        let effective_env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        for secret_key in ["BUZZ_PRIVATE_KEY", "NOSTR_PRIVATE_KEY", "BUZZ_AUTH_TAG"] {
            assert_eq!(effective_env.get(secret_key), Some(&None));
        }
        assert_eq!(
            effective_env
                .get("BUZZ_ACP_CREDENTIAL_STDIN")
                .and_then(|value| value.as_deref()),
            Some("true")
        );
        let decoded: serde_json::Value =
            serde_json::from_slice(&payload).expect("decode protected credential payload");
        assert_eq!(decoded["private_key"], "private-canary");
        assert_eq!(decoded["auth_tag"], "auth-canary");
    }

    #[test]
    fn first_save_stacy_tuple_injects_only_stacy_pins_and_protected_stdin() {
        let agent = "12".repeat(32);
        let owner = "34".repeat(32);
        let class = classify_context_engine_harness(
            TRUSTED_CONTEXT_ENGINE_NODE,
            &[TRUSTED_STACY_CONTEXT_ENGINE_ADAPTER.to_string()],
        );
        let ContextEngineHarnessClass::Exact(harness) = class else {
            panic!("reviewed Stacy tuple must classify as exact");
        };
        let outer_acp = resolve_outer_acp_command(class, "buzz-acp", |_| {
            panic!("the Add Agent default must not resolve through PATH for a trusted tuple")
        })
        .expect("trusted Stacy tuple selects the frozen outer ACP binary");
        assert_eq!(outer_acp, std::path::Path::new(TRUSTED_BUZZ_ACP));
        let mut command = std::process::Command::new("/usr/bin/true");
        for key in CONTEXT_ENGINE_IDENTITY_KEYS {
            command.env(key, "ambient-imposter");
        }
        command
            .env("BUZZ_PRIVATE_KEY", "private-canary")
            .env("NOSTR_PRIVATE_KEY", "nostr-canary")
            .env("BUZZ_AUTH_TAG", "auth-canary");
        apply_context_engine_identity(&mut command, Some(harness), &agent, Some(&owner))
            .expect("inject exact Stacy identity");
        let _payload =
            install_acp_credential_stdin(&mut command, "private-canary", Some("auth-canary"))
                .expect("install protected first-save credentials");
        let env = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value.map(|value| value.to_string_lossy().into_owned()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            env["BUZZ_STACY_AGENT_PUBKEY"].as_deref(),
            Some(agent.as_str())
        );
        assert_eq!(
            env["BUZZ_STACY_OWNER_PUBKEY"].as_deref(),
            Some(owner.as_str())
        );
        assert_eq!(env["BUZZ_GABE_AGENT_PUBKEY"], None);
        assert_eq!(env["BUZZ_GABE_OWNER_PUBKEY"], None);
        for secret_key in ["BUZZ_PRIVATE_KEY", "NOSTR_PRIVATE_KEY", "BUZZ_AUTH_TAG"] {
            assert_eq!(env[secret_key], None);
        }
        assert_eq!(env["BUZZ_ACP_CREDENTIAL_STDIN"].as_deref(), Some("true"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn credential_stdin_is_absent_from_macos_process_inspection() {
        use sha2::{Digest as _, Sha256};
        use std::process::Stdio;
        use std::time::Duration;

        const PRIVATE_CANARY: &str = "STACY_STDIN_PRIVATE_CANARY_6f7c30";
        const AUTH_CANARY: &str = "STACY_STDIN_AUTH_CANARY_4d093e";
        let expected_private_hash = hex::encode(Sha256::digest(PRIVATE_CANARY.as_bytes()));
        let expected_auth_hash = hex::encode(Sha256::digest(AUTH_CANARY.as_bytes()));
        let script = r#"import hashlib,json,sys,time
payload=json.load(sys.stdin)
assert hashlib.sha256(payload['private_key'].encode()).hexdigest() == sys.argv[1]
assert hashlib.sha256(payload['auth_tag'].encode()).hexdigest() == sys.argv[2]
time.sleep(2)
"#;
        let mut command = std::process::Command::new("/usr/bin/python3");
        command
            .args(["-c", script, &expected_private_hash, &expected_auth_hash])
            .env("BUZZ_PRIVATE_KEY", PRIVATE_CANARY)
            .env("NOSTR_PRIVATE_KEY", PRIVATE_CANARY)
            .env("BUZZ_AUTH_TAG", AUTH_CANARY)
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let payload = install_acp_credential_stdin(&mut command, PRIVATE_CANARY, Some(AUTH_CANARY))
            .expect("install protected credential stdin");
        let mut child = command.spawn().expect("spawn credential inspection probe");
        deliver_acp_credentials(&mut child, &payload).expect("deliver protected credentials");

        std::thread::sleep(Duration::from_millis(200));
        assert!(child.try_wait().expect("probe child status").is_none());
        let inspected = std::process::Command::new("/bin/ps")
            .args(["eww", "-p", &child.id().to_string()])
            .output()
            .expect("inspect probe argv and environment");
        let surface = String::from_utf8_lossy(&inspected.stdout);
        assert!(!surface.contains(PRIVATE_CANARY));
        assert!(!surface.contains(AUTH_CANARY));

        let _ = child.kill();
        let _ = child.wait();
    }
}
