//! Pure child-process environment configuration.

use super::*;

pub(crate) fn build_respond_to_env(
    record: &ManagedAgentRecord,
    owner_hex: Option<&str>,
) -> Result<RespondToEnv, String> {
    let normalized =
        super::super::types::validate_respond_to_allowlist(&record.respond_to_allowlist)?;
    if record.respond_to == super::super::types::RespondTo::Allowlist && normalized.is_empty() {
        return Err(
            "respond-to mode 'allowlist' requires at least one pubkey in the allowlist".to_string(),
        );
    }
    let mut set = vec![(
        "BUZZ_ACP_RESPOND_TO",
        record.respond_to.as_str().to_string(),
    )];
    let mut remove = Vec::new();
    if record.respond_to == super::super::types::RespondTo::Allowlist {
        set.push(("BUZZ_ACP_RESPOND_TO_ALLOWLIST", normalized.join(",")));
    } else {
        remove.push("BUZZ_ACP_RESPOND_TO_ALLOWLIST");
    }
    if record.auth_tag.is_none() {
        if let Some(owner) = owner_hex {
            set.push(("BUZZ_ACP_AGENT_OWNER", owner.to_string()));
        } else {
            remove.push("BUZZ_ACP_AGENT_OWNER");
        }
    } else {
        remove.push("BUZZ_ACP_AGENT_OWNER");
    }
    Ok((set, remove))
}

pub(crate) fn configure_runtime_cli(
    command: &mut std::process::Command,
    runtime: Option<&KnownAcpRuntime>,
) {
    let Some(runtime) = runtime else {
        return;
    };
    if runtime.id != "claude" {
        return;
    }
    if let Some(cli_path) = runtime.underlying_cli.and_then(resolve_command) {
        if !should_skip_claude_executable(&cli_path, cfg!(windows)) {
            command.env("CLAUDE_CODE_EXECUTABLE", cli_path);
        }
    }
}
