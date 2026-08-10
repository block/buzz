use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{
    missing_command_message, resolve_command, CreateManagedAgentRequest, ManagedAgentRecord,
    UpdateManagedAgentRequest,
};

/// Optional executable placed between `buzz-acp` and the selected ACP runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentCommandWrapper {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
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

    /// Whether this wrapper is the managed Nxtlinq authorization boundary.
    /// This is derived from structured launch configuration, never Agent env.
    pub fn is_nxtlinq_gateway(&self) -> bool {
        let command_path = PathBuf::from(self.command.trim());
        let basename = command_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .trim_end_matches(".cmd")
            .trim_end_matches(".exe")
            .trim_end_matches(".bat");
        basename == "nxtlinq-authorization-gateway"
    }

    pub fn apply_nxtlinq_trust(wrapper: Option<&Self>, command: &mut std::process::Command) {
        command.env_remove("BUZZ_ACP_TRUST_NXTLINQ_GATEWAY");
        if wrapper.is_some_and(Self::is_nxtlinq_gateway) {
            command.env("BUZZ_ACP_TRUST_NXTLINQ_GATEWAY", "1");
        }
    }
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
        };
        let error = compose_wrapped_launch(&wrapper, "/usr/bin/agent".into(), &[])
            .expect_err("comma must fail closed");
        assert!(error.contains("cannot contain commas"));
    }

    #[test]
    fn nxtlinq_identity_is_derived_from_the_wrapper_not_environment() {
        let wrapper = AgentCommandWrapper {
            command: "/managed/bin/nxtlinq-authorization-gateway".into(),
            args: Vec::new(),
        };
        assert!(wrapper.is_nxtlinq_gateway());
        let unrelated = AgentCommandWrapper {
            command: "/managed/bin/buzz-agent".into(),
            args: Vec::new(),
        };
        assert!(!unrelated.is_nxtlinq_gateway());
    }
}
