use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;
#[cfg(windows)]
use std::time::{Duration, Instant};

use super::discovery::{
    find_command, known_acp_runtime, missing_command_message, normalize_command_identity,
    resolve_command,
};
use super::AcpAvailabilityStatus;
use super::KnownAcpRuntime;

pub(crate) struct ResolvedRuntimeAdapter {
    pub command: &'static str,
    pub path: PathBuf,
    pub launch_command: &'static str,
}

pub(crate) fn resolve_runtime_adapter(
    runtime: &'static KnownAcpRuntime,
) -> Option<ResolvedRuntimeAdapter> {
    let direct = runtime.commands.iter().find_map(|command| {
        #[cfg(windows)]
        if runtime.native_acp {
            return None;
        }
        let path = find_command(command)?;
        Some(ResolvedRuntimeAdapter {
            command: *command,
            path,
            launch_command: *command,
        })
    });
    if direct.is_some() || !runtime.native_acp {
        return direct;
    }

    #[cfg(windows)]
    {
        let wsl = find_command("wsl.exe")?;
        for &guest_command in runtime.commands {
            for probe in [["--version"], ["--help"]] {
                if run_wsl_probe(&wsl, guest_command, &probe) {
                    return Some(ResolvedRuntimeAdapter {
                        command: "wsl.exe",
                        path: wsl,
                        launch_command: guest_command,
                    });
                }
            }
        }
        None
    }

    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
fn run_wsl_probe(wsl: &PathBuf, guest_command: &str, probe: &[&str]) -> bool {
    let mut child = match Command::new(wsl)
        .args(["--cd", "~", "--"])
        .arg(guest_command)
        .args(probe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return false,
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub(crate) fn runtime_launch_args(
    runtime: &KnownAcpRuntime,
    command: &str,
    launch_command: &str,
    mut args: Vec<String>,
) -> Vec<String> {
    if !runtime.native_acp
        || !matches!(
            normalize_command_identity(command).as_str(),
            "agent" | "cursor-agent" | "wsl"
        )
    {
        return args;
    }
    if !args.iter().any(|arg| arg.eq_ignore_ascii_case("acp")) {
        args.push("acp".to_string());
    }
    if normalize_command_identity(command) == "wsl" {
        let mut launch_args = vec![
            "--cd".to_string(),
            "~".to_string(),
            "--".to_string(),
            launch_command.to_string(),
        ];
        launch_args.extend(args);
        launch_args
    } else {
        args
    }
}

pub(crate) fn startup_model_args(
    runtime: &KnownAcpRuntime,
    command: &str,
    launch_command: &str,
    args: Vec<String>,
    model: Option<&str>,
) -> Vec<String> {
    let mut args = runtime_launch_args(runtime, command, launch_command, args);
    let Some(model) = model else {
        return args;
    };
    let Some(model_arg) = runtime.startup_model_arg else {
        return args;
    };
    let mut filtered = Vec::with_capacity(args.len() + 2);
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == model_arg {
            skip_next = true;
            continue;
        }
        if arg
            .strip_prefix(model_arg)
            .is_some_and(|suffix| suffix.starts_with('='))
        {
            continue;
        }
        filtered.push(arg);
    }
    args = filtered;
    let insert_at = args
        .iter()
        .position(|arg| arg.eq_ignore_ascii_case("acp"))
        .unwrap_or(args.len());
    args.splice(
        insert_at..insert_at,
        [model_arg.to_string(), model.to_string()],
    );
    args
}

pub(crate) fn resolve_known_runtime_launch(
    command: &str,
    args: Vec<String>,
) -> Option<(String, Vec<String>)> {
    let runtime = known_acp_runtime(command)?;
    let adapter = resolve_runtime_adapter(runtime)?;
    Some((
        adapter.path.display().to_string(),
        runtime_launch_args(runtime, adapter.command, adapter.launch_command, args),
    ))
}

pub(crate) fn resolve_agent_launch(
    command: &str,
    args: Vec<String>,
) -> Result<(String, Vec<String>), String> {
    if known_acp_runtime(command).is_some() {
        return resolve_known_runtime_launch(command, args)
            .ok_or_else(|| missing_command_message(command, "agent CLI"));
    }
    Ok((
        resolve_command(command)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| command.to_string()),
        args,
    ))
}

pub(crate) fn classify_acp_runtime(
    runtime: &KnownAcpRuntime,
    adapter_result: Option<&ResolvedRuntimeAdapter>,
    underlying_cli_found: bool,
) -> (AcpAvailabilityStatus, Option<String>, Option<String>) {
    if runtime.native_acp {
        return match adapter_result {
            Some(adapter) => (
                AcpAvailabilityStatus::Available,
                Some(adapter.command.to_string()),
                Some(adapter.path.display().to_string()),
            ),
            None => (AcpAvailabilityStatus::NotInstalled, None, None),
        };
    }
    super::discovery::classify_runtime(
        adapter_result.map(|adapter| (adapter.command, adapter.path.clone())),
        runtime.underlying_cli,
        underlying_cli_found,
    )
}

pub(crate) fn native_auth_probe_args(command: &str, launch_command: &str) -> Vec<String> {
    if normalize_command_identity(command) == "wsl" {
        vec![
            command.to_string(),
            "--cd".to_string(),
            "~".to_string(),
            "--".to_string(),
            launch_command.to_string(),
            "status".to_string(),
        ]
    } else {
        vec![command.to_string(), "status".to_string()]
    }
}

pub(crate) fn resolve_runtime_launch(
    runtime: Option<&'static KnownAcpRuntime>,
    command: &str,
    args: Vec<String>,
    model: Option<&str>,
) -> Option<(String, Vec<String>)> {
    if let Some(runtime) = runtime {
        let adapter = resolve_runtime_adapter(runtime)?;
        return Some((
            adapter.path.display().to_string(),
            startup_model_args(runtime, adapter.command, adapter.launch_command, args, model),
        ));
    }
    Some((
        resolve_command(command)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| command.to_string()),
        args,
    ))
}

#[cfg(test)]
mod tests {
    use super::{startup_model_args, runtime_launch_args};
    use crate::managed_agents::known_acp_runtime_exact;

    #[test]
    fn startup_model_args_replace_space_and_equals_forms() {
        let runtime = known_acp_runtime_exact("cursor").expect("Cursor metadata");
        let args = startup_model_args(
            runtime,
            "agent",
            "agent",
            vec![
                "--model".into(),
                "old-one".into(),
                "--model=old-two".into(),
                "acp".into(),
            ],
            Some("new-model"),
        );
        assert_eq!(
            args,
            vec![
                "--model".to_string(),
                "new-model".to_string(),
                "acp".to_string()
            ]
        );
    }

    #[test]
    fn runtime_launch_args_add_acp_once() {
        let runtime = known_acp_runtime_exact("cursor").expect("Cursor metadata");
        assert_eq!(
            runtime_launch_args(runtime, "agent", "agent", vec!["acp".into()]),
            vec!["acp".to_string()]
        );
    }
}
