use std::ffi::OsString;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::managed_agents::{
    validate_heartbeat_preflight_configuration, HeartbeatHarnessStamp, ManagedAgentPairRuntime,
    ManagedAgentRecord, ManagedAgentRuntimeKey, DEFAULT_ACP_COMMAND,
};

const CONFIG_ENV: &str = "BUZZ_ACP_HEARTBEAT_PREFLIGHT_CONFIG";
const REQUIRED_ENV: &str = "BUZZ_ACP_HEARTBEAT_PREFLIGHT_REQUIRED";
const POLICY_FILE_ENV: &str = "BUZZ_ACP_HEARTBEAT_PREFLIGHT_POLICY_FILE";
const POLICY_SHA256_ENV: &str = "BUZZ_ACP_HEARTBEAT_PREFLIGHT_POLICY_SHA256";
const HEARTBEAT_INTERVAL_ENV: &str = "BUZZ_ACP_HEARTBEAT_INTERVAL";
const CAPABILITY_COMMAND: &str = "heartbeat-preflight-capability";
const CAPABILITY_KIND: &str = "buzz_acp_heartbeat_preflight_capability";
const CAPABILITY_PROTOCOL_VERSION: u32 = 1;
const BUILD_CAPABILITY: &str = "buzz-acp-source-witness-gateway-v1";
#[cfg(target_os = "macos")]
const TRUSTED_MACOS_HARNESS_PATH: &str =
    "/Library/Application Support/Buzz/TrustedHeartbeat/buzz-acp";
const CONTROL_ENV_KEYS: &[&str] = &[
    CONFIG_ENV,
    REQUIRED_ENV,
    POLICY_FILE_ENV,
    POLICY_SHA256_ENV,
    HEARTBEAT_INTERVAL_ENV,
];

#[derive(Clone, Debug, Eq, PartialEq)]
struct FileIdentity {
    len: u64,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl FileIdentity {
    fn from_metadata(metadata: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Self {
                len: metadata.len(),
                dev: metadata.dev(),
                ino: metadata.ino(),
            }
        }
        #[cfg(not(unix))]
        Self {
            len: metadata.len(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct VerifiedHeartbeatHarness {
    pub(super) path: PathBuf,
    pub(super) stamp: HeartbeatHarnessStamp,
    identity: FileIdentity,
}

struct HarnessExpectation {
    path: PathBuf,
    binary_sha256: String,
}

trait HarnessResolver {
    fn resolve(&self) -> Result<HarnessExpectation, String>;

    fn requires_platform_authenticity(&self) -> bool {
        false
    }
}

struct DesignatedHarnessResolver;

impl HarnessResolver for DesignatedHarnessResolver {
    fn resolve(&self) -> Result<HarnessExpectation, String> {
        #[cfg(target_os = "macos")]
        let path = PathBuf::from(TRUSTED_MACOS_HARNESS_PATH);
        #[cfg(not(target_os = "macos"))]
        let path = {
            let executable = std::env::current_exe()
                .map_err(|error| format!("cannot locate this Desktop build: {error}"))?;
            let directory = executable
                .parent()
                .ok_or_else(|| "Desktop executable has no parent directory".to_string())?;
            #[cfg(windows)]
            let filename = "buzz-acp.exe";
            #[cfg(not(windows))]
            let filename = "buzz-acp";
            directory.join(filename)
        };
        let binary_sha256 = option_env!("BUZZ_DESKTOP_BUNDLED_BUZZ_ACP_SHA256")
            .filter(|digest| is_lower_hex(digest, 64))
            .ok_or_else(|| {
                "this Desktop build has no exact bundled buzz-acp identity pin".to_string()
            })?;
        Ok(HarnessExpectation {
            path,
            binary_sha256: binary_sha256.to_string(),
        })
    }

    fn requires_platform_authenticity(&self) -> bool {
        true
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HarnessCapability {
    kind: String,
    protocol_version: u32,
    build_capability: String,
}

#[derive(Serialize)]
struct DesignationAuthority<'a> {
    schema_version: u32,
    target_agent_pubkey: &'a str,
    backend: &'static str,
    acp_command: &'a str,
    policy_file: &'a str,
    policy_sha256: &'a str,
    heartbeat_interval_seconds: u64,
}

/// Stable digest for every owner-authoritative input that determines whether
/// an existing heartbeat harness is safe to reuse.
fn designation_authority_sha256(record: &ManagedAgentRecord) -> Result<String, String> {
    let designation = record
        .heartbeat_preflight
        .as_ref()
        .ok_or_else(|| "heartbeat preflight designation is missing".to_string())?;
    let policy_file = designation
        .policy_file
        .to_str()
        .ok_or_else(|| "heartbeat preflight policy file must be valid UTF-8".to_string())?;
    let canonical = DesignationAuthority {
        schema_version: 1,
        target_agent_pubkey: &record.pubkey,
        backend: "local",
        acp_command: &record.acp_command,
        policy_file,
        policy_sha256: &designation.policy_sha256,
        heartbeat_interval_seconds: designation.heartbeat_interval_seconds,
    };
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| format!("cannot encode heartbeat preflight designation: {error}"))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

trait HarnessProber {
    fn probe(&self, path: &Path) -> Result<HarnessCapability, String>;
}

struct ProcessHarnessProber;

impl HarnessProber for ProcessHarnessProber {
    fn probe(&self, path: &Path) -> Result<HarnessCapability, String> {
        let mut child = Command::new(path)
            .arg(CAPABILITY_COMMAND)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("cannot run bundled buzz-acp capability probe: {error}"))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if child
                .try_wait()
                .map_err(|error| format!("cannot wait for buzz-acp capability probe: {error}"))?
                .is_some()
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err("bundled buzz-acp capability probe timed out".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("cannot read buzz-acp capability probe: {error}"))?;
        if !output.status.success() || output.stdout.len() > 4_096 || !output.stderr.is_empty() {
            return Err("bundled buzz-acp capability probe failed closed".into());
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("bundled buzz-acp capability is invalid: {error}"))
    }
}

pub(super) fn verify(
    record: &ManagedAgentRecord,
) -> Result<Option<VerifiedHeartbeatHarness>, String> {
    verify_with(record, &DesignatedHarnessResolver, &ProcessHarnessProber)
}

fn current_stamp(record: &ManagedAgentRecord) -> Result<Option<HeartbeatHarnessStamp>, String> {
    verify(record).map(|verified| verified.map(|verified| verified.stamp))
}

pub(crate) fn reuse_if_verified(
    app: &tauri::AppHandle,
    record: &mut ManagedAgentRecord,
    runtimes: &mut std::collections::HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>,
    key: &ManagedAgentRuntimeKey,
) -> Result<bool, String> {
    let running_stamp = match runtimes.get_mut(key) {
        None => return Ok(false),
        Some(runtime) => match runtime
            .child
            .try_wait()
            .map_err(|error| format!("failed to inspect running process: {error}"))?
        {
            None => runtime.heartbeat_harness.clone(),
            Some(_) => return Ok(false),
        },
    };
    let verified = current_stamp(record);
    if verified.as_ref().is_ok_and(|stamp| *stamp == running_stamp) {
        return Ok(true);
    }
    super::stop_managed_agent_pair(app, record, runtimes, key)?;
    verified.map(|_| false)
}

fn verify_with<R: HarnessResolver, P: HarnessProber>(
    record: &ManagedAgentRecord,
    resolver: &R,
    prober: &P,
) -> Result<Option<VerifiedHeartbeatHarness>, String> {
    let Some(designation) = record.heartbeat_preflight.as_ref() else {
        return Ok(None);
    };
    validate_heartbeat_preflight_configuration(
        Some(designation),
        &record.backend,
        &record.acp_command,
        &record.pubkey,
    )?;
    if record.acp_command != DEFAULT_ACP_COMMAND {
        return Err("designated heartbeat preflight requires bundled buzz-acp".into());
    }

    let expected = resolver.resolve()?;
    let identity = validate_path_and_hash(&expected.path, &expected.binary_sha256).map_err(|error| {
        #[cfg(target_os = "macos")]
        {
            let revision = option_env!("BUZZ_DESKTOP_SOURCE_REVISION")
                .filter(|revision| {
                    matches!(revision.len(), 40 | 64)
                        && revision.bytes().all(|byte| {
                            byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                        })
                })
                .unwrap_or("unavailable-source-revision");
            format!(
                "{error}; an administrator must refresh this Buzz build's trusted heartbeat harness using its immutable macOS procedure: https://github.com/block/buzz/blob/{revision}/desktop/README.md#trusted-heartbeat-harness-on-macos"
            )
        }
        #[cfg(not(target_os = "macos"))]
        {
            error
        }
    })?;
    #[cfg(target_os = "macos")]
    if resolver.requires_platform_authenticity() {
        validate_macos_harness_authenticity(&expected.path)?;
    }
    let capability = prober.probe(&expected.path)?;
    if capability.kind != CAPABILITY_KIND
        || capability.protocol_version != CAPABILITY_PROTOCOL_VERSION
        || capability.build_capability != BUILD_CAPABILITY
    {
        return Err("bundled buzz-acp lacks the exact heartbeat-preflight capability".into());
    }
    if validate_path_and_hash(&expected.path, &expected.binary_sha256)? != identity {
        return Err("bundled buzz-acp changed during its capability probe".into());
    }
    #[cfg(target_os = "macos")]
    if resolver.requires_platform_authenticity() {
        validate_macos_harness_authenticity(&expected.path)?;
    }
    Ok(Some(VerifiedHeartbeatHarness {
        path: expected.path,
        stamp: HeartbeatHarnessStamp {
            binary_sha256: expected.binary_sha256,
            protocol_version: capability.protocol_version,
            build_capability: capability.build_capability,
            designation_sha256: designation_authority_sha256(record)?,
        },
        identity,
    }))
}

fn validate_path_and_hash(path: &Path, expected_sha256: &str) -> Result<FileIdentity, String> {
    validate_path_and_hash_with_ownership(path, expected_sha256, cfg!(not(test)))
}

fn validate_path_and_hash_with_ownership(
    path: &Path,
    expected_sha256: &str,
    require_root_owner: bool,
) -> Result<FileIdentity, String> {
    #[cfg(not(unix))]
    if require_root_owner {
        return Err("designated heartbeat harnesses require an immutable Unix package path".into());
    }
    if !path.is_absolute() {
        return Err("bundled buzz-acp path is not absolute".into());
    }
    let mut current = PathBuf::new();
    let components: Vec<_> = path.components().collect();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::Normal(name) => current.push(name),
            Component::CurDir | Component::ParentDir => {
                return Err("bundled buzz-acp path contains traversal".into());
            }
        }
        let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
            format!(
                "cannot inspect bundled buzz-acp path {}: {error}",
                current.display()
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "bundled buzz-acp path component {} is a symlink",
                current.display()
            ));
        }
        #[cfg(target_os = "macos")]
        if require_root_owner {
            reject_macos_extended_acl(&current)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if require_root_owner && metadata.uid() != 0 {
                return Err(format!(
                    "bundled buzz-acp path component {} is not root-owned",
                    current.display()
                ));
            }
            if metadata.permissions().mode() & 0o022 != 0 {
                return Err(format!(
                    "bundled buzz-acp path component {} is group/world writable",
                    current.display()
                ));
            }
        }
        if index + 1 == components.len() {
            if !metadata.file_type().is_file() {
                return Err("bundled buzz-acp is not a regular file".into());
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o111 == 0 {
                    return Err("bundled buzz-acp is not executable".into());
                }
            }
            let checked_identity = FileIdentity::from_metadata(&metadata);
            let mut file = std::fs::File::open(path)
                .map_err(|error| format!("cannot open bundled buzz-acp: {error}"))?;
            let opened_identity = FileIdentity::from_metadata(
                &file
                    .metadata()
                    .map_err(|error| format!("cannot inspect open buzz-acp: {error}"))?,
            );
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)
                .map_err(|error| format!("cannot hash bundled buzz-acp: {error}"))?;
            let actual_sha256 =
                crate::managed_agents::binary_identity::executable_identity_sha256(&bytes)?;
            if opened_identity != checked_identity
                || actual_sha256 != expected_sha256
                || FileIdentity::from_metadata(
                    &std::fs::symlink_metadata(path)
                        .map_err(|error| format!("cannot recheck bundled buzz-acp: {error}"))?,
                ) != checked_identity
            {
                return Err("bundled buzz-acp does not match this Desktop build".into());
            }
            return Ok(checked_identity);
        }
    }
    Err("bundled buzz-acp path has no file component".into())
}

#[cfg(target_os = "macos")]
fn reject_macos_extended_acl(path: &Path) -> Result<(), String> {
    use std::os::unix::ffi::OsStrExt;

    if path
        .as_os_str()
        .as_bytes()
        .iter()
        .any(|byte| matches!(byte, b'\n' | b'\r'))
    {
        return Err(format!(
            "heartbeat harness path {} cannot be inspected safely for extended ACLs",
            path.display()
        ));
    }
    let output = Command::new("/bin/ls")
        .args(["-lde"])
        .arg(path)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            format!(
                "cannot inspect extended ACL for heartbeat harness path {}: {error}",
                path.display()
            )
        })?;
    if !output.status.success() || !output.stderr.is_empty() || output.stdout.len() > 65_536 {
        return Err(format!(
            "cannot inspect extended ACL for heartbeat harness path {}",
            path.display()
        ));
    }
    let report = std::str::from_utf8(&output.stdout).map_err(|error| {
        format!(
            "cannot decode extended ACL report for heartbeat harness path {}: {error}",
            path.display()
        )
    })?;
    let mut lines = report.lines();
    let summary = lines.next().ok_or_else(|| {
        format!(
            "extended ACL report for heartbeat harness path {} is empty",
            path.display()
        )
    })?;
    let acl_marker = summary
        .split_ascii_whitespace()
        .next()
        .is_some_and(|permissions| permissions.ends_with('+'));
    if acl_marker || lines.next().is_some() {
        return Err(format!(
            "heartbeat harness path component {} has an extended ACL",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_macos_harness_authenticity(path: &Path) -> Result<(), String> {
    let team = option_env!("BUZZ_DESKTOP_HEARTBEAT_HARNESS_MACOS_TEAM_IDENTIFIER")
        .filter(|team| {
            team.len() == 10
                && team
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        })
        .ok_or_else(|| {
            "this Desktop build has no trusted heartbeat-harness TeamIdentifier pin".to_string()
        })?;
    let requirement = format!(
        "identifier \"buzz-acp\" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */ and certificate leaf[subject.OU] = \"{team}\""
    );
    let verification = Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "--test-requirement", &requirement])
        .arg(path)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot authenticate heartbeat harness signature: {error}"))?;
    if !verification.status.success() {
        return Err("heartbeat harness signature does not match this Buzz build".into());
    }

    let details = Command::new("/usr/bin/codesign")
        .args(["--display", "--verbose=4"])
        .arg(path)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot inspect heartbeat harness signature: {error}"))?;
    if !details.status.success() {
        return Err("cannot inspect heartbeat harness signing policy".into());
    }
    validate_macos_signature_report(&String::from_utf8_lossy(&details.stderr), team)?;

    let entitlements = Command::new("/usr/bin/codesign")
        .args(["--display", "--entitlements", "-", "--xml"])
        .arg(path)
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("cannot inspect heartbeat harness entitlements: {error}"))?;
    if !entitlements.status.success() || !entitlements.stdout.is_empty() {
        return Err("heartbeat harness must not carry entitlement exceptions".into());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn validate_macos_signature_report(report: &str, expected_team: &str) -> Result<(), String> {
    let identifier = report
        .lines()
        .find_map(|line| line.strip_prefix("Identifier="));
    let team = report
        .lines()
        .find_map(|line| line.strip_prefix("TeamIdentifier="));
    let flags = report
        .split_whitespace()
        .find_map(|field| field.strip_prefix("flags=0x"))
        .and_then(|value| value.split('(').next())
        .and_then(|value| u32::from_str_radix(value, 16).ok());
    if identifier != Some("buzz-acp") || team != Some(expected_team) {
        return Err("heartbeat harness signing identity is not exact".into());
    }
    if flags.is_none_or(|flags| flags & 0x0001_0000 == 0) {
        return Err("heartbeat harness is missing hardened runtime".into());
    }
    Ok(())
}

/// Apply the durable Desktop designation after all layered user env has been
/// written. Every ambient or preconfigured spelling of a control key is
/// removed before the exact owner cadence and policy are set.
pub(super) fn configure_env(
    command: &mut Command,
    record: &ManagedAgentRecord,
) -> Result<(), String> {
    remove_case_variants(command, CONTROL_ENV_KEYS);
    if let Some(designation) = record.heartbeat_preflight.as_ref() {
        validate_heartbeat_preflight_configuration(
            Some(designation),
            &record.backend,
            &record.acp_command,
            &record.pubkey,
        )?;
        command
            .env(REQUIRED_ENV, "true")
            .env(POLICY_FILE_ENV, &designation.policy_file)
            .env(POLICY_SHA256_ENV, &designation.policy_sha256)
            .env(
                HEARTBEAT_INTERVAL_ENV,
                designation.heartbeat_interval_seconds.to_string(),
            );
    } else if let Some(value) = std::env::var_os(CONFIG_ENV).filter(|value| !value.is_empty()) {
        command.env(CONFIG_ENV, value);
    }
    Ok(())
}

fn remove_case_variants(command: &mut Command, reserved: &[&str]) {
    let mut keys: Vec<OsString> = std::env::vars_os().map(|(key, _)| key).collect();
    keys.extend(command.get_envs().map(|(key, _)| key.to_os_string()));
    for key in keys {
        if key.to_str().is_some_and(|key| {
            reserved
                .iter()
                .any(|reserved| reserved.eq_ignore_ascii_case(key))
        }) {
            command.env_remove(key);
        }
    }
    for key in reserved {
        command.env_remove(key);
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    struct Resolver(HarnessExpectation);
    impl HarnessResolver for Resolver {
        fn resolve(&self) -> Result<HarnessExpectation, String> {
            Ok(HarnessExpectation {
                path: self.0.path.clone(),
                binary_sha256: self.0.binary_sha256.clone(),
            })
        }
    }

    struct Prober(HarnessCapability);
    impl HarnessProber for Prober {
        fn probe(&self, _path: &Path) -> Result<HarnessCapability, String> {
            Ok(HarnessCapability {
                kind: self.0.kind.clone(),
                protocol_version: self.0.protocol_version,
                build_capability: self.0.build_capability.clone(),
            })
        }
    }

    struct PanicResolver;
    impl HarnessResolver for PanicResolver {
        fn resolve(&self) -> Result<HarnessExpectation, String> {
            panic!("resolver must not run for an invalid owner cadence")
        }
    }

    struct PanicProber;
    impl HarnessProber for PanicProber {
        fn probe(&self, _path: &Path) -> Result<HarnessCapability, String> {
            panic!("probe must not run for an invalid owner cadence")
        }
    }

    fn record(policy: &Path, policy_sha256: String) -> ManagedAgentRecord {
        let mut record: ManagedAgentRecord = serde_json::from_value(serde_json::json!({
            "pubkey": "a".repeat(64),
            "name": "test-agent",
            "private_key_nsec": "nsec1test",
            "relay_url": "",
            "acp_command": "buzz-acp",
            "agent_command": "buzz-agent",
            "agent_args": [],
            "mcp_command": "",
            "turn_timeout_seconds": 320,
            "system_prompt": null,
            "model": null,
            "provider": null,
            "env_vars": {},
            "created_at": "",
            "updated_at": "",
            "last_started_at": null,
            "last_stopped_at": null,
            "last_exit_code": null,
            "last_error": null
        }))
        .expect("minimal managed-agent fixture");
        record.heartbeat_preflight = Some(crate::managed_agents::HeartbeatPreflightDesignation {
            policy_file: policy.to_path_buf(),
            policy_sha256,
            heartbeat_interval_seconds: 3_600,
        });
        record
    }

    fn fixture() -> (tempfile::TempDir, PathBuf, ManagedAgentRecord, Resolver) {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(directory.path()).expect("canonical tempdir");
        let policy = root.join("policy.json");
        let policy_bytes = serde_json::to_vec(&serde_json::json!({
            "target_agent_pubkey": "a".repeat(64),
            "heartbeat_interval_seconds": 3600,
        }))
        .expect("policy json");
        std::fs::write(&policy, &policy_bytes).expect("write policy");
        let binary = root.join("buzz-acp");
        std::fs::write(&binary, b"exact test binary").expect("write binary");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))
            .expect("executable");
        let binary_sha256 = hex::encode(Sha256::digest(b"exact test binary"));
        let resolver = Resolver(HarnessExpectation {
            path: binary,
            binary_sha256,
        });
        let record = record(&policy, hex::encode(Sha256::digest(&policy_bytes)));
        (directory, resolver.0.path.clone(), record, resolver)
    }

    fn exact_prober() -> Prober {
        Prober(HarnessCapability {
            kind: CAPABILITY_KIND.into(),
            protocol_version: CAPABILITY_PROTOCOL_VERSION,
            build_capability: BUILD_CAPABILITY.into(),
        })
    }

    #[test]
    fn capable_exact_binary_is_verified_and_stamped() {
        let (_directory, _binary, record, resolver) = fixture();
        let verified = verify_with(&record, &resolver, &exact_prober())
            .expect("verify")
            .expect("designated");
        assert_eq!(verified.stamp.binary_sha256, resolver.0.binary_sha256);
        assert_eq!(verified.stamp.protocol_version, 1);
        assert!(is_lower_hex(&verified.stamp.designation_sha256, 64));
    }

    #[test]
    fn policy_path_only_change_changes_runtime_stamp() {
        let (_directory, binary, record, resolver) = fixture();
        let first = verify_with(&record, &resolver, &exact_prober())
            .expect("verify first")
            .expect("designated")
            .stamp;
        let second_policy = binary
            .parent()
            .expect("binary parent")
            .join("policy-2.json");
        std::fs::copy(
            record
                .heartbeat_preflight
                .as_ref()
                .expect("designation")
                .policy_file
                .as_path(),
            &second_policy,
        )
        .expect("copy policy");
        let mut changed = record;
        changed
            .heartbeat_preflight
            .as_mut()
            .expect("designation")
            .policy_file = second_policy;
        let second = verify_with(&changed, &resolver, &exact_prober())
            .expect("verify changed policy path")
            .expect("designated")
            .stamp;
        assert_ne!(first, second, "policy path changes must prevent reuse");
    }

    #[test]
    fn policy_digest_only_change_changes_runtime_stamp() {
        let (_directory, _binary, mut record, resolver) = fixture();
        let first = verify_with(&record, &resolver, &exact_prober())
            .expect("verify first policy")
            .expect("designated")
            .stamp;
        let changed_policy = serde_json::to_vec_pretty(&serde_json::json!({
            "target_agent_pubkey": "a".repeat(64),
            "heartbeat_interval_seconds": 3600,
        }))
        .expect("changed policy bytes");
        let designation = record.heartbeat_preflight.as_mut().expect("designation");
        std::fs::write(&designation.policy_file, &changed_policy).expect("write changed policy");
        designation.policy_sha256 = hex::encode(Sha256::digest(&changed_policy));
        let second = verify_with(&record, &resolver, &exact_prober())
            .expect("verify changed policy digest")
            .expect("designated")
            .stamp;

        assert_eq!(first.binary_sha256, second.binary_sha256);
        assert_ne!(
            first.designation_sha256, second.designation_sha256,
            "policy byte changes must prevent reuse"
        );
    }

    #[test]
    fn cadence_only_change_changes_runtime_stamp() {
        let (_directory, _binary, mut record, resolver) = fixture();
        let first = verify_with(&record, &resolver, &exact_prober())
            .expect("verify first cadence")
            .expect("designated")
            .stamp;
        let changed_policy = serde_json::to_vec(&serde_json::json!({
            "target_agent_pubkey": "a".repeat(64),
            "heartbeat_interval_seconds": 3601,
        }))
        .expect("changed policy json");
        let designation = record.heartbeat_preflight.as_mut().expect("designation");
        std::fs::write(&designation.policy_file, &changed_policy).expect("write changed policy");
        designation.policy_sha256 = hex::encode(Sha256::digest(&changed_policy));
        designation.heartbeat_interval_seconds = 3_601;
        let second = verify_with(&record, &resolver, &exact_prober())
            .expect("verify changed cadence")
            .expect("designated")
            .stamp;

        assert_eq!(first.binary_sha256, second.binary_sha256);
        assert_eq!(first.protocol_version, second.protocol_version);
        assert_eq!(first.build_capability, second.build_capability);
        assert_ne!(
            first.designation_sha256, second.designation_sha256,
            "cadence changes must prevent reuse"
        );
    }

    #[test]
    fn old_capability_stub_is_rejected_even_when_hash_matches() {
        let (_directory, _binary, record, resolver) = fixture();
        let old = Prober(HarnessCapability {
            kind: CAPABILITY_KIND.into(),
            protocol_version: 0,
            build_capability: "old-stub".into(),
        });
        assert!(verify_with(&record, &resolver, &old)
            .expect_err("old stub must fail")
            .contains("exact heartbeat-preflight capability"));
    }

    #[test]
    fn symlinked_bundle_binary_is_rejected_before_probe() {
        let (_directory, binary, record, mut resolver) = fixture();
        let link = binary
            .parent()
            .expect("binary parent")
            .join("buzz-acp-link");
        symlink(&binary, &link).expect("symlink");
        resolver.0.path = link;
        assert!(verify_with(&record, &resolver, &exact_prober())
            .expect_err("symlink must fail")
            .contains("symlink"));
    }

    #[test]
    fn user_owned_bundle_path_is_rejected_for_designated_production_launch() {
        let (_directory, binary, _record, resolver) = fixture();
        let error = validate_path_and_hash_with_ownership(&binary, &resolver.0.binary_sha256, true)
            .expect_err("user-owned harness path must fail closed");
        assert!(
            error.contains("not root-owned")
                || error.contains("group/world writable")
                || error.contains("extended ACL")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn designated_macos_launch_resolves_only_the_privileged_install() {
        let resolved = DesignatedHarnessResolver
            .resolve()
            .expect("build digest pin");
        assert_eq!(resolved.path, PathBuf::from(TRUSTED_MACOS_HARNESS_PATH));
        assert!(!resolved.path.starts_with("/Applications"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_extended_acl_is_rejected() {
        let directory = tempfile::tempdir().expect("temporary ACL directory");
        let path = directory.path().join("acl-bearing-harness");
        std::fs::write(&path, b"harness").expect("write ACL fixture");
        reject_macos_extended_acl(&path).expect("ordinary file has no extended ACL");
        let status = Command::new("/bin/chmod")
            .args(["+a", "everyone allow write"])
            .arg(&path)
            .status()
            .expect("apply extended ACL");
        assert!(status.success(), "extended ACL fixture must be created");
        assert!(reject_macos_extended_acl(&path)
            .expect_err("ACL-bearing harness must fail closed")
            .contains("extended ACL"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_signature_report_requires_exact_identity_and_hardened_runtime() {
        let report = "Identifier=buzz-acp\nCodeDirectory v=20500 flags=0x10000(runtime) hashes=1+1 location=embedded\nTeamIdentifier=EYF346PHUG\n";
        validate_macos_signature_report(report, "EYF346PHUG").expect("exact signature report");
        assert!(validate_macos_signature_report(report, "AAAAAAAAAA").is_err());
        assert!(validate_macos_signature_report(
            "Identifier=buzz-acp\nCodeDirectory v=20500 flags=0x0(none)\nTeamIdentifier=EYF346PHUG\n",
            "EYF346PHUG",
        )
        .expect_err("hardened runtime is mandatory")
        .contains("hardened runtime"));
    }

    #[test]
    fn mixed_case_ambient_cadence_is_removed_and_exact_value_wins() {
        let (_directory, _binary, record, _resolver) = fixture();
        let mut command = Command::new("ignored");
        command
            .env("buzz_acp_heartbeat_interval", "1")
            .env(HEARTBEAT_INTERVAL_ENV, "2");
        configure_env(&mut command, &record).expect("configure");
        let env: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
            .collect();
        assert_eq!(
            env.get(std::ffi::OsStr::new(HEARTBEAT_INTERVAL_ENV)),
            Some(&Some("3600".into()))
        );
        assert_eq!(
            env.get(std::ffi::OsStr::new("buzz_acp_heartbeat_interval")),
            Some(&None)
        );
    }

    #[test]
    fn persisted_designation_restores_exact_child_environment() {
        let (_directory, _binary, record, _resolver) = fixture();
        let bytes = serde_json::to_vec(&record).expect("persist record");
        let restored: ManagedAgentRecord =
            serde_json::from_slice(&bytes).expect("restore record after restart");
        let designation = restored
            .heartbeat_preflight
            .as_ref()
            .expect("persisted designation")
            .clone();
        let mut command = Command::new("ignored");
        command
            .env(CONFIG_ENV, "/forged/config.json")
            .env("buzz_acp_heartbeat_preflight_required", "false")
            .env(POLICY_SHA256_ENV, "0".repeat(64));
        configure_env(&mut command, &restored).expect("configure restored child");
        let env: std::collections::BTreeMap<_, _> = command
            .get_envs()
            .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
            .collect();

        assert_eq!(
            env.get(std::ffi::OsStr::new(REQUIRED_ENV)),
            Some(&Some("true".into()))
        );
        assert_eq!(
            env.get(std::ffi::OsStr::new(POLICY_FILE_ENV)),
            Some(&Some(designation.policy_file.into_os_string()))
        );
        assert_eq!(
            env.get(std::ffi::OsStr::new(POLICY_SHA256_ENV)),
            Some(&Some(designation.policy_sha256.into()))
        );
        assert_eq!(
            env.get(std::ffi::OsStr::new(HEARTBEAT_INTERVAL_ENV)),
            Some(&Some("3600".into()))
        );
        assert_eq!(env.get(std::ffi::OsStr::new(CONFIG_ENV)), Some(&None));
        assert_eq!(
            env.get(std::ffi::OsStr::new(
                "buzz_acp_heartbeat_preflight_required"
            )),
            Some(&None)
        );
    }

    #[test]
    fn zero_or_out_of_range_cadence_refuses_before_harness_resolution() {
        for seconds in [0, 86_401] {
            let (_directory, _binary, mut record, _resolver) = fixture();
            record.heartbeat_preflight =
                Some(crate::managed_agents::HeartbeatPreflightDesignation {
                    policy_file: "/missing/policy.json".into(),
                    policy_sha256: "a".repeat(64),
                    heartbeat_interval_seconds: seconds,
                });
            assert!(verify_with(&record, &PanicResolver, &PanicProber)
                .expect_err("invalid cadence must prevent spawn")
                .contains("interval must be between"));
        }
    }
}
