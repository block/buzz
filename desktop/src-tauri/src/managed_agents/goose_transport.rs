use std::process::Command;

use super::discovery::KnownAcpRuntime;

pub(super) fn apply_native_provider(command: &mut Command, runtime: Option<&KnownAcpRuntime>) {
    if runtime.is_none_or(|runtime| runtime.id != "goose") {
        return;
    }
    if let Some(native_provider) = super::config_bridge::read_goose_file_config()
        .and_then(|config| config.provider)
        .filter(|provider| !provider.trim().is_empty())
    {
        command.env("GOOSE_PROVIDER", native_provider);
    }
}
