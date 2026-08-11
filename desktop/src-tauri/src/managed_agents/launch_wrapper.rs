use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::{
    missing_command_message, resolve_command, CreateManagedAgentRequest, ManagedAgentRecord,
    UpdateManagedAgentRequest,
};

pub(crate) const NXTLINQ_GATEWAY_COMMAND: &str = "nxtlinq-authorization-gateway";
pub(crate) const NXTLINQ_GATEWAY_PACKAGE: &str = "@nxtlinq/authorization-gateway";
pub(crate) const NXTLINQ_GATEWAY_VERSION: &str = "0.3.0";

/// Host verification attached to a wrapper by an authorization preset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum AgentCommandWrapperAuthorization {
    NxtlinqGateway { executable: String, sha256: String },
}

/// Optional executable placed between `buzz-acp` and the selected ACP runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCommandWrapper {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Binding produced by the Host readiness check. Its executable must still
    /// resolve to Buzz's reviewed managed package at every process spawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) authorization: Option<AgentCommandWrapperAuthorization>,
}

impl AgentCommandWrapper {
    /// Validate and normalize an operator-supplied wrapper.
    pub fn normalized(mut self) -> Result<Self, String> {
        self.command = self.command.trim().to_string();
        if self.command.is_empty() {
            return Err("agent command wrapper requires a non-empty command".to_string());
        }
        if self.command.contains('\0') || self.args.iter().any(|arg| arg.contains('\0')) {
            return Err(
                "agent command wrapper command and args cannot contain NUL bytes".to_string(),
            );
        }
        Ok(self)
    }

    fn matches_verified_nxtlinq_gateway(&self, verified: &VerifiedNxtlinqGateway) -> bool {
        let Some(AgentCommandWrapperAuthorization::NxtlinqGateway { executable, sha256 }) =
            self.authorization.as_ref()
        else {
            return false;
        };
        canonical_file_identity(std::path::Path::new(self.command.trim()))
            == Some(verified.canonical_executable.clone())
            && canonical_file_identity(std::path::Path::new(executable))
                == Some(verified.canonical_executable.clone())
            && sha256 == &verified.executable_sha256
    }

    pub fn apply_nxtlinq_trust(wrapper: Option<&Self>, command: &mut std::process::Command) {
        command.env_remove("BUZZ_ACP_TRUST_NXTLINQ_GATEWAY");
        let trusted = wrapper.is_some_and(|wrapper| {
            verify_managed_nxtlinq_gateway()
                .is_ok_and(|verified| wrapper.matches_verified_nxtlinq_gateway(&verified))
        });
        if trusted {
            command.env("BUZZ_ACP_TRUST_NXTLINQ_GATEWAY", "1");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedNxtlinqGateway {
    pub(crate) executable: PathBuf,
    canonical_executable: PathBuf,
    pub(crate) executable_sha256: String,
    pub(crate) version: String,
}

#[derive(Deserialize)]
struct NxtlinqPackageManifest {
    name: String,
    version: String,
}

fn canonical_file_identity(path: &std::path::Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn nxtlinq_package_root(prefix: &std::path::Path) -> PathBuf {
    #[cfg(windows)]
    {
        prefix
            .join("node_modules")
            .join("@nxtlinq")
            .join("authorization-gateway")
    }
    #[cfg(not(windows))]
    {
        prefix
            .join("lib")
            .join("node_modules")
            .join("@nxtlinq")
            .join("authorization-gateway")
    }
}

fn verify_nxtlinq_gateway_at(
    executable: &std::path::Path,
    managed_bin: &std::path::Path,
    package_root: &std::path::Path,
) -> Result<VerifiedNxtlinqGateway, String> {
    let executable_parent = executable
        .parent()
        .ok_or_else(|| "Gateway executable has no parent directory".to_string())?;
    let managed_bin = std::fs::canonicalize(managed_bin)
        .map_err(|error| format!("could not resolve Buzz managed npm bin directory: {error}"))?;
    let executable_parent = std::fs::canonicalize(executable_parent)
        .map_err(|error| format!("could not resolve Gateway executable directory: {error}"))?;
    if executable_parent != managed_bin {
        return Err("Gateway executable is outside Buzz's managed npm directory".to_string());
    }

    let canonical_executable = std::fs::canonicalize(executable)
        .map_err(|error| format!("could not resolve Gateway executable: {error}"))?;
    let executable_bytes = std::fs::read(&canonical_executable)
        .map_err(|error| format!("could not read Gateway executable: {error}"))?;
    let executable_sha256 = hex::encode(Sha256::digest(executable_bytes));
    let canonical_package_root = std::fs::canonicalize(package_root)
        .map_err(|error| format!("could not resolve installed Gateway package: {error}"))?;
    #[cfg(not(windows))]
    if !canonical_executable.starts_with(&canonical_package_root) {
        return Err("Gateway executable does not resolve into the managed package".to_string());
    }

    let manifest_path = canonical_package_root.join("package.json");
    let manifest_bytes = std::fs::read(&manifest_path)
        .map_err(|error| format!("could not read {}: {error}", manifest_path.display()))?;
    let manifest: NxtlinqPackageManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("could not parse {}: {error}", manifest_path.display()))?;
    if manifest.name != NXTLINQ_GATEWAY_PACKAGE {
        return Err(format!(
            "managed executable belongs to {}, not {}",
            manifest.name, NXTLINQ_GATEWAY_PACKAGE
        ));
    }
    if manifest.version != NXTLINQ_GATEWAY_VERSION {
        return Err(format!(
            "Gateway version mismatch: expected {}, found {}",
            NXTLINQ_GATEWAY_VERSION, manifest.version
        ));
    }

    Ok(VerifiedNxtlinqGateway {
        executable: executable.to_path_buf(),
        canonical_executable,
        executable_sha256,
        version: manifest.version,
    })
}

pub(crate) fn verify_managed_nxtlinq_gateway() -> Result<VerifiedNxtlinqGateway, String> {
    let executable = resolve_command(NXTLINQ_GATEWAY_COMMAND)
        .ok_or_else(|| "Nxtlinq Gateway is not installed".to_string())?;
    let managed_bin = super::buzz_managed_npm_bin_dir()
        .ok_or_else(|| "Buzz managed npm directory is unavailable".to_string())?;
    let prefix = super::buzz_managed_npm_prefix()
        .ok_or_else(|| "Buzz managed npm prefix is unavailable".to_string())?;
    verify_nxtlinq_gateway_at(&executable, &managed_bin, &nxtlinq_package_root(&prefix))
}

/// Validate the optional ACP workspace selected by the operator.
pub(crate) fn validate_agent_working_directory(
    path: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.is_absolute() {
        return Err("agent working directory must be an absolute path".to_string());
    }
    if !path.is_dir() {
        return Err(format!(
            "agent working directory does not exist or is not a directory: {}",
            path.display()
        ));
    }
    Ok(Some(path))
}

pub(crate) fn normalized_agent_launch_config(
    input: &CreateManagedAgentRequest,
) -> Result<(Option<AgentCommandWrapper>, Option<PathBuf>), String> {
    Ok((
        input
            .command_wrapper
            .clone()
            .map(AgentCommandWrapper::normalized)
            .transpose()?,
        validate_agent_working_directory(input.working_directory.clone())?,
    ))
}

pub(crate) fn apply_agent_launch_update(
    input: &UpdateManagedAgentRequest,
    record: &mut ManagedAgentRecord,
) -> Result<(), String> {
    if let Some(command_wrapper) = &input.command_wrapper {
        record.command_wrapper = command_wrapper
            .clone()
            .map(AgentCommandWrapper::normalized)
            .transpose()?;
    }
    if let Some(working_directory) = &input.working_directory {
        record.working_directory = validate_agent_working_directory(working_directory.clone())?;
    }
    Ok(())
}

pub(crate) struct AgentLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: Option<PathBuf>,
}

impl AgentLaunch {
    pub fn apply_working_directory(&self, command: &mut std::process::Command) {
        if let Some(workdir) = self
            .working_directory
            .clone()
            .or_else(super::default_agent_workdir)
        {
            command.current_dir(workdir);
        }
    }
}

/// Compose the shell-free process driven by `buzz-acp`.
pub(crate) fn resolve_agent_launch(
    record: &ManagedAgentRecord,
    downstream_command: String,
    downstream_args: &[String],
) -> Result<AgentLaunch, String> {
    let working_directory = validate_agent_working_directory(record.working_directory.clone())?;
    let (command, args) = match &record.command_wrapper {
        None => (downstream_command, downstream_args.to_vec()),
        Some(wrapper) => compose_wrapped_launch(wrapper, downstream_command, downstream_args)?,
    };
    Ok(AgentLaunch {
        command,
        args,
        working_directory,
    })
}

fn compose_wrapped_launch(
    wrapper: &AgentCommandWrapper,
    downstream_command: String,
    downstream_args: &[String],
) -> Result<(String, Vec<String>), String> {
    let command = resolve_command(&wrapper.command)
        .map(|path| path.display().to_string())
        .ok_or_else(|| missing_command_message(&wrapper.command, "agent command wrapper"))?;
    let mut args = Vec::with_capacity(wrapper.args.len() + downstream_args.len() + 1);
    args.extend(wrapper.args.iter().cloned());
    args.push(downstream_command);
    args.extend(downstream_args.iter().cloned());
    if args.iter().any(|arg| arg.contains(',')) {
        return Err(
            "agent command wrapper arguments cannot contain commas because buzz-acp currently transports argv as a comma-delimited value"
                .to_string(),
        );
    }
    Ok((command, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapper_preserves_downstream_runtime_and_argv() {
        let wrapper_command = std::env::current_exe().expect("current test executable");
        let wrapper = AgentCommandWrapper {
            command: wrapper_command.display().to_string(),
            args: vec!["nxtlinq-authorization-gateway".into(), "--".into()],
            authorization: None,
        };
        let (command, args) =
            compose_wrapped_launch(&wrapper, "/usr/local/bin/goose".into(), &["acp".into()])
                .expect("wrapper composition");
        assert_eq!(command, wrapper_command.display().to_string());
        assert_eq!(
            args,
            vec![
                "nxtlinq-authorization-gateway",
                "--",
                "/usr/local/bin/goose",
                "acp"
            ]
        );
    }

    #[test]
    fn wrapper_rejects_comma_that_would_change_argv() {
        let wrapper = AgentCommandWrapper {
            command: std::env::current_exe()
                .expect("current test executable")
                .display()
                .to_string(),
            args: vec!["value,with-comma".into()],
            authorization: None,
        };
        let error = compose_wrapped_launch(&wrapper, "/usr/bin/agent".into(), &[])
            .expect_err("comma must fail closed");
        assert!(error.contains("cannot contain commas"));
    }

    #[test]
    fn nxtlinq_trust_requires_the_exact_verified_executable() {
        let executable = std::env::current_exe().expect("current test executable");
        let canonical_executable = executable.canonicalize().expect("canonical executable");
        let verified = VerifiedNxtlinqGateway {
            executable: executable.clone(),
            canonical_executable,
            executable_sha256: hex::encode(Sha256::digest(
                std::fs::read(&executable).expect("read current test executable"),
            )),
            version: NXTLINQ_GATEWAY_VERSION.into(),
        };
        let wrapper = AgentCommandWrapper {
            command: executable.display().to_string(),
            args: Vec::new(),
            authorization: Some(AgentCommandWrapperAuthorization::NxtlinqGateway {
                executable: executable.display().to_string(),
                sha256: verified.executable_sha256.clone(),
            }),
        };
        assert!(wrapper.matches_verified_nxtlinq_gateway(&verified));

        let substituted = AgentCommandWrapper {
            command: "/tmp/nxtlinq-authorization-gateway".into(),
            ..wrapper.clone()
        };
        assert!(!substituted.matches_verified_nxtlinq_gateway(&verified));

        let unverified = AgentCommandWrapper {
            authorization: None,
            ..wrapper.clone()
        };
        assert!(!unverified.matches_verified_nxtlinq_gateway(&verified));

        let changed_bytes = AgentCommandWrapper {
            authorization: Some(AgentCommandWrapperAuthorization::NxtlinqGateway {
                executable: executable.display().to_string(),
                sha256: "0".repeat(64),
            }),
            ..wrapper
        };
        assert!(!changed_bytes.matches_verified_nxtlinq_gateway(&verified));
    }

    #[test]
    fn nxtlinq_verification_binding_round_trips_with_the_wrapper() {
        let wrapper = AgentCommandWrapper {
            command: "/managed/bin/nxtlinq-authorization-gateway".into(),
            args: vec!["--adapter".into(), "acp".into(), "--".into()],
            authorization: Some(AgentCommandWrapperAuthorization::NxtlinqGateway {
                executable: "/managed/bin/nxtlinq-authorization-gateway".into(),
                sha256: "a".repeat(64),
            }),
        };
        let encoded = serde_json::to_value(&wrapper).expect("serialize wrapper");
        assert_eq!(encoded["authorization"]["kind"], "nxtlinq_gateway");
        assert_eq!(encoded["authorization"]["sha256"], "a".repeat(64));
        let decoded: AgentCommandWrapper =
            serde_json::from_value(encoded).expect("deserialize wrapper");
        assert_eq!(decoded, wrapper);
    }

    #[cfg(unix)]
    #[test]
    fn managed_gateway_verification_rejects_version_mismatch_and_substitution() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temp directory");
        let prefix = root.path().join("node-tools");
        let managed_bin = prefix.join("bin");
        let package_root = nxtlinq_package_root(&prefix);
        let package_bin = package_root.join("bin");
        std::fs::create_dir_all(&managed_bin).expect("managed bin");
        std::fs::create_dir_all(&package_bin).expect("package bin");
        let package_executable = package_bin.join("nxtlinq-authorization-gateway.mjs");
        std::fs::write(&package_executable, "#!/usr/bin/env node\n").expect("package executable");
        std::fs::write(
            package_root.join("package.json"),
            format!(
                r#"{{"name":"{}","version":"{}"}}"#,
                NXTLINQ_GATEWAY_PACKAGE, NXTLINQ_GATEWAY_VERSION
            ),
        )
        .expect("package manifest");
        let launch_path = managed_bin.join(NXTLINQ_GATEWAY_COMMAND);
        symlink(&package_executable, &launch_path).expect("managed symlink");

        assert!(verify_nxtlinq_gateway_at(&launch_path, &managed_bin, &package_root).is_ok());

        std::fs::write(
            package_root.join("package.json"),
            format!(
                r#"{{"name":"{}","version":"9.9.9"}}"#,
                NXTLINQ_GATEWAY_PACKAGE
            ),
        )
        .expect("mismatched manifest");
        let mismatch = verify_nxtlinq_gateway_at(&launch_path, &managed_bin, &package_root)
            .expect_err("version mismatch must fail closed");
        assert!(mismatch.contains("version mismatch"));

        let outside = root.path().join("substitute.mjs");
        std::fs::write(&outside, "#!/usr/bin/env node\n").expect("substitute");
        std::fs::remove_file(&launch_path).expect("remove managed symlink");
        symlink(&outside, &launch_path).expect("substitute symlink");
        let substitution = verify_nxtlinq_gateway_at(&launch_path, &managed_bin, &package_root)
            .expect_err("substitution must fail closed");
        assert!(substitution.contains("does not resolve into the managed package"));
    }
}
