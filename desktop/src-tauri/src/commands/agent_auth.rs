use std::{
    path::Path,
    process::{Command, Stdio},
};

#[cfg(target_os = "macos")]
use std::{fs, io::Write, os::unix::fs::PermissionsExt};

use serde_json::Value;

use serde::{Deserialize, Serialize};

use crate::managed_agents::{
    default_agent_workdir, known_acp_runtime_exact, normalize_agent_args, resolve_command,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AcpAuthMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub method_type: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// Full terminal command advertised by the adapter. Buzz never guesses
    /// vendor login commands; when present, this argv is the source of truth.
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default, rename = "_meta")]
    pub meta: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AcpAuthMethodsResult {
    pub methods: Vec<AcpAuthMethod>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectAcpRuntimeRequest {
    pub runtime_id: String,
    pub method_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectAcpRuntimeResult {
    pub launched: bool,
}

#[tauri::command]
pub async fn discover_acp_auth_methods(runtime_id: String) -> Result<AcpAuthMethodsResult, String> {
    tokio::task::spawn_blocking(move || discover_acp_auth_methods_blocking(&runtime_id))
        .await
        .map_err(|error| format!("auth-method discovery task failed: {error}"))?
}

#[tauri::command]
pub async fn connect_acp_runtime(
    request: ConnectAcpRuntimeRequest,
) -> Result<ConnectAcpRuntimeResult, String> {
    tokio::task::spawn_blocking(move || connect_acp_runtime_blocking(&request))
        .await
        .map_err(|error| format!("connect-account task failed: {error}"))?
}

fn discover_acp_auth_methods_blocking(runtime_id: &str) -> Result<AcpAuthMethodsResult, String> {
    let output = run_buzz_acp_auth_command(runtime_id, ["auth-methods", "--json"])?;
    if !output.status.success() {
        return Err(command_error("buzz-acp auth-methods", &output));
    }

    serde_json::from_slice::<AcpAuthMethodsResult>(&output.stdout)
        .map_err(|error| format!("failed to parse auth methods JSON: {error}"))
}

fn connect_acp_runtime_blocking(
    request: &ConnectAcpRuntimeRequest,
) -> Result<ConnectAcpRuntimeResult, String> {
    let methods = discover_acp_auth_methods_blocking(&request.runtime_id)?;
    let method = methods
        .methods
        .iter()
        .find(|candidate| candidate.id == request.method_id)
        .ok_or_else(|| "auth method is no longer advertised by this adapter".to_string())?;

    if uses_terminal_auth(method)? {
        if is_claude_subscription_login(&request.runtime_id, method) {
            run_claude_subscription_login(&request.runtime_id, method)?;
        } else {
            launch_terminal_auth(&request.runtime_id, method)?;
        }
        return Ok(ConnectAcpRuntimeResult { launched: true });
    }

    let output = run_buzz_acp_auth_command(
        &request.runtime_id,
        ["authenticate", "--method-id", request.method_id.as_str()],
    )?;
    if !output.status.success() {
        return Err(command_error("buzz-acp authenticate", &output));
    }

    Ok(ConnectAcpRuntimeResult { launched: true })
}

fn run_buzz_acp_auth_command<const N: usize>(
    runtime_id: &str,
    args: [&str; N],
) -> Result<std::process::Output, String> {
    let runtime = known_acp_runtime_exact(runtime_id)
        .ok_or_else(|| format!("unknown ACP runtime: {runtime_id}"))?;
    let adapter_command = runtime
        .commands
        .iter()
        .find_map(|command| resolve_command(command).map(|path| (*command, path)))
        .ok_or_else(|| format!("{} ACP adapter is not installed", runtime.label))?;

    let acp_path = std::env::current_exe()
        .map(|path| path.with_file_name(format!("buzz-acp{}", std::env::consts::EXE_SUFFIX)))
        .ok()
        .filter(|path| path.exists())
        .or_else(|| resolve_command("buzz-acp"))
        .ok_or_else(|| "buzz-acp helper not found".to_string())?;

    let augmented_path = auth_command_path();
    run_buzz_acp_auth_command_with_paths(
        &acp_path,
        adapter_command.0,
        &adapter_command.1,
        args,
        augmented_path.as_deref(),
    )
}

/// PATH for the buzz-acp auth helper child process.
///
/// Uses the augmented agent PATH so `#!/usr/bin/env node` adapter shims
/// resolve the Buzz-managed Node runtime — the same PATH normal agent
/// launches and readiness probes use.
///
/// On Windows, `login_shell_path()` is intentionally `None`, so the augmented
/// PATH contains only Buzz-managed directories and the exe parent. Buzz does
/// not ship a managed Node runtime on Windows, and npm `.cmd` adapters need
/// the user's normal PATH to find `node` (and often `claude`/`codex`), so the
/// inherited process PATH is appended there instead of being replaced.
fn auth_command_path() -> Option<String> {
    let augmented = crate::managed_agents::readiness::cli_probe::augmented_path();
    if !cfg!(windows) {
        return augmented;
    }
    let inherited = std::env::var("PATH").ok().filter(|path| !path.is_empty());
    append_inherited_path(augmented, inherited)
}

/// Append `inherited` PATH entries after `augmented` ones. Returns `None`
/// when `augmented` is `None` so the child simply inherits the process
/// environment unchanged (the pre-augmentation behavior on Windows).
fn append_inherited_path(augmented: Option<String>, inherited: Option<String>) -> Option<String> {
    let augmented = augmented?;
    let Some(inherited) = inherited else {
        return Some(augmented);
    };
    let parts: Vec<std::path::PathBuf> = std::env::split_paths(&augmented)
        .chain(std::env::split_paths(&inherited))
        .collect();
    std::env::join_paths(parts)
        .ok()
        .map(|joined| joined.to_string_lossy().into_owned())
        .or(Some(augmented))
}

fn run_buzz_acp_auth_command_with_paths<const N: usize>(
    acp_path: &Path,
    adapter_name: &str,
    adapter_path: &Path,
    args: [&str; N],
    augmented_path: Option<&str>,
) -> Result<std::process::Output, String> {
    let agent_args = normalize_agent_args(adapter_name, Vec::new());
    let mut command = Command::new(acp_path);
    command
        .args(args)
        .env("BUZZ_ACP_AGENT_COMMAND", adapter_path.as_os_str())
        .env("BUZZ_ACP_AGENT_ARGS", agent_args.join(","))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(workdir) = default_agent_workdir() {
        command.current_dir(workdir);
    }
    if let Some(path) = augmented_path {
        command.env("PATH", path);
    }

    command
        .output()
        .map_err(|error| format!("failed to run buzz-acp auth helper: {error}"))
}

fn command_error(label: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!(
            "{label} failed (exit {})",
            output.status.code().unwrap_or(-1)
        )
    } else {
        format!(
            "{label} failed (exit {}): {stderr}",
            output.status.code().unwrap_or(-1)
        )
    }
}

fn is_claude_subscription_login(runtime_id: &str, method: &AcpAuthMethod) -> bool {
    runtime_id == "claude" && matches!(method.id.as_str(), "claude-login" | "claude-ai-login")
}

fn run_claude_subscription_login(runtime_id: &str, method: &AcpAuthMethod) -> Result<(), String> {
    let runtime = known_acp_runtime_exact(runtime_id)
        .ok_or_else(|| format!("unknown ACP runtime: {runtime_id}"))?;
    let argv = adapter_terminal_argv(runtime.label, method, "")?;
    let (command, args) = argv
        .split_first()
        .ok_or_else(|| "Claude login command is empty".to_string())?;
    let status = Command::new(command)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("failed to run Claude login: {error}"))?;
    if !status.success() {
        return Err(format!(
            "Claude login failed (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

fn launch_terminal_auth(runtime_id: &str, method: &AcpAuthMethod) -> Result<(), String> {
    let runtime = known_acp_runtime_exact(runtime_id)
        .ok_or_else(|| format!("unknown ACP runtime: {runtime_id}"))?;
    let adapter_command = runtime
        .commands
        .iter()
        .find_map(|command| resolve_command(command).map(|path| (*command, path)))
        .ok_or_else(|| format!("{} ACP adapter is not installed", runtime.label))?;
    let fallback_command = adapter_command.1.display().to_string();
    let argv = adapter_terminal_argv(runtime.label, method, &fallback_command)?;
    launch_visible_terminal(&argv)
}

fn adapter_terminal_argv(
    runtime_label: &str,
    method: &AcpAuthMethod,
    fallback_command: &str,
) -> Result<Vec<String>, String> {
    let meta_command = terminal_auth_meta_command(method)?;
    let (command, args): (&str, &[String]) =
        match meta_command.as_deref().and_then(|argv| argv.split_first()) {
            Some((command, args)) => (command.as_str(), args),
            None => match method.command.split_first() {
                Some((command, args)) => (command.as_str(), args),
                None => (fallback_command, method.args.as_slice()),
            },
        };

    if command.trim().is_empty() {
        return Err(format!(
            "{} did not provide a terminal login command for {}",
            runtime_label, method.name
        ));
    }

    let command_path = resolve_command(command)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| command.to_string());
    let mut argv = vec![command_path];
    argv.extend(args.iter().cloned());
    if method.id == "claude-login" && terminal_auth_meta_command(method)?.is_some() {
        argv.extend(["auth".to_string(), "login".to_string()]);
    }
    Ok(argv)
}

fn uses_terminal_auth(method: &AcpAuthMethod) -> Result<bool, String> {
    Ok(method.method_type.as_deref() == Some("terminal")
        || terminal_auth_meta_command(method)?.is_some())
}

fn terminal_auth_meta_command(method: &AcpAuthMethod) -> Result<Option<Vec<String>>, String> {
    let Some(meta) = method.meta.as_ref() else {
        return Ok(None);
    };
    let Some(terminal_auth) = meta.get("terminal-auth") else {
        return Ok(None);
    };
    let Some(command) = terminal_auth.get("command") else {
        return Ok(None);
    };

    if let Some(command) = command.as_str() {
        let mut argv = vec![command.to_string()];
        if let Some(args) = terminal_auth.get("args") {
            let args = args.as_array().ok_or_else(|| {
                format!(
                    "terminal auth metadata for {} has non-array args",
                    method.name
                )
            })?;
            for value in args {
                let Some(arg) = value.as_str() else {
                    return Err(format!(
                        "terminal auth metadata for {} has a non-string arg",
                        method.name
                    ));
                };
                argv.push(arg.to_string());
            }
        }
        return Ok((!argv.is_empty()).then_some(argv));
    }

    let command = command.as_array().ok_or_else(|| {
        format!(
            "terminal auth metadata for {} has a non-string/non-array command",
            method.name
        )
    })?;
    let mut argv = Vec::with_capacity(command.len());
    for value in command {
        let Some(arg) = value.as_str() else {
            return Err(format!(
                "terminal auth metadata for {} has a non-string command argument",
                method.name
            ));
        };
        argv.push(arg.to_string());
    }
    Ok((!argv.is_empty()).then_some(argv))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn spawn_without_stdio(mut command: Command) -> Result<(), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to open terminal: {error}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn launch_visible_terminal(argv: &[String]) -> Result<(), String> {
    let mut script = tempfile::Builder::new()
        .prefix("buzz-auth-")
        .suffix(".command")
        .tempfile()
        .map_err(|error| format!("failed to create terminal login script: {error}"))?;
    writeln!(
        script,
        "#!/bin/sh\ntrap 'rm -f -- \"$0\"' EXIT\n{}",
        shell_join(argv)
    )
    .map_err(|error| format!("failed to write terminal login script: {error}"))?;
    fs::set_permissions(script.path(), fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to prepare terminal login script: {error}"))?;
    let script = script
        .into_temp_path()
        .keep()
        .map_err(|error| format!("failed to preserve terminal login script: {error}"))?;

    let status = Command::new("open")
        .arg(&script)
        .status()
        .map_err(|error| format!("failed to open terminal: {error}"))?;
    if !status.success() {
        let _ = fs::remove_file(&script);
        return Err(format!(
            "failed to open terminal (exit {})",
            status.code().unwrap_or(-1)
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn launch_visible_terminal(argv: &[String]) -> Result<(), String> {
    let command = shell_join(argv);
    let candidates: [(&str, &[&str]); 4] = [
        ("x-terminal-emulator", &["-e", "sh", "-lc"]),
        ("gnome-terminal", &["--", "sh", "-lc"]),
        ("konsole", &["-e", "sh", "-lc"]),
        ("xterm", &["-e", "sh", "-lc"]),
    ];
    for (terminal, prefix) in candidates {
        let mut terminal_command = Command::new(terminal);
        terminal_command.args(prefix).arg(&command);
        if spawn_without_stdio(terminal_command).is_ok() {
            return Ok(());
        }
    }
    Err("no terminal emulator found".to_string())
}

#[cfg(target_os = "windows")]
fn launch_visible_terminal(argv: &[String]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    let mut command = Command::new("cmd");
    // Keep argv separate so Rust applies Windows command-line quoting. Joining
    // with POSIX shell escaping breaks paths such as `C:\Program Files\...`.
    command
        .args(windows_terminal_args(argv))
        .creation_flags(CREATE_NEW_CONSOLE);
    spawn_without_stdio(command)
}

#[cfg(any(target_os = "windows", test))]
fn windows_terminal_args(argv: &[String]) -> Vec<String> {
    std::iter::once("/K".to_string())
        .chain(argv.iter().cloned())
        .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn launch_visible_terminal(_argv: &[String]) -> Result<(), String> {
    Err("opening a terminal is not supported on this platform".to_string())
}

fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| shell_escape(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_escape(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '-' | '.' | ':' | '='))
    {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::{
        adapter_terminal_argv, append_inherited_path, is_claude_subscription_login,
        run_buzz_acp_auth_command_with_paths, shell_escape, shell_join, uses_terminal_auth,
        windows_terminal_args, AcpAuthMethod,
    };

    /// Windows regression: the augmented PATH there holds only Buzz-managed
    /// dirs and the exe parent (no login-shell PATH, no managed Node), so the
    /// user's inherited PATH must be appended for npm `.cmd` adapters to find
    /// `node`/`claude`/`codex`.
    #[test]
    fn append_inherited_path_appends_after_augmented() {
        let sep = if cfg!(windows) { ';' } else { ':' };
        let augmented = format!("{0}buzz-bin{1}{0}exe-dir", std::path::MAIN_SEPARATOR, sep);
        let inherited = format!(
            "{0}user-bin{1}{0}system-bin",
            std::path::MAIN_SEPARATOR,
            sep
        );
        let joined = append_inherited_path(Some(augmented.clone()), Some(inherited.clone()))
            .expect("joined PATH");
        assert_eq!(joined, format!("{augmented}{sep}{inherited}"));
    }

    #[test]
    fn append_inherited_path_falls_back_to_inheritance_without_augmented() {
        // With no augmented PATH the helper must not set PATH at all, letting
        // the child inherit the process environment unchanged.
        assert_eq!(append_inherited_path(None, Some("anything".into())), None);
    }

    #[test]
    fn append_inherited_path_keeps_augmented_without_inherited() {
        assert_eq!(
            append_inherited_path(Some("only-augmented".into()), None),
            Some("only-augmented".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn auth_command_uses_augmented_path_for_node_adapter() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("temp dir");
        let interpreter_dir = temp.path().join("interpreter-bin");
        fs::create_dir_all(&interpreter_dir).expect("interpreter dir");

        let marker_path = temp.path().join("fake-node-ran");
        let node_path = interpreter_dir.join("node");
        fs::write(
            &node_path,
            format!(
                "#!/bin/sh\nprintf 'fake node ran\\n' > '{}' || exit 1\nprintf '{{\"methods\":[]}}\\n'\n",
                marker_path.display()
            ),
        )
        .expect("write fake node");
        fs::set_permissions(&node_path, fs::Permissions::from_mode(0o755))
            .expect("chmod fake node");

        let adapter_path = temp.path().join("claude-agent-acp");
        fs::write(&adapter_path, "#!/usr/bin/env node\n").expect("write adapter");
        fs::set_permissions(&adapter_path, fs::Permissions::from_mode(0o755))
            .expect("chmod adapter");

        let acp_path = temp.path().join("buzz-acp");
        fs::write(&acp_path, "#!/bin/sh\nexec \"$BUZZ_ACP_AGENT_COMMAND\"\n")
            .expect("write buzz-acp");
        fs::set_permissions(&acp_path, fs::Permissions::from_mode(0o755)).expect("chmod buzz-acp");

        let augmented_path = std::env::join_paths([interpreter_dir.as_path()])
            .expect("join augmented PATH")
            .to_string_lossy()
            .into_owned();
        let output = run_buzz_acp_auth_command_with_paths(
            &acp_path,
            "claude-agent-acp",
            &adapter_path,
            ["auth-methods", "--json"],
            Some(&augmented_path),
        )
        .expect("run auth command");

        assert!(
            output.status.success(),
            "auth command should find node through augmented PATH: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(marker_path.exists(), "the fake node interpreter should run");
        assert_eq!(
            output.stdout,
            br#"{"methods":[]}
"#
        );
    }

    #[test]
    fn shell_join_escapes_spaces_and_quotes() {
        assert_eq!(
            shell_join(&["/bin/claude".into(), "auth login".into(), "it's".into()]),
            "/bin/claude 'auth login' 'it'\\''s'"
        );
    }

    #[test]
    fn shell_escape_leaves_simple_args_unquoted() {
        assert_eq!(shell_escape("--claudeai"), "--claudeai");
    }

    #[test]
    fn windows_terminal_keeps_argv_separate() {
        let argv = vec![
            r"C:\Program Files\Codex\codex.exe".to_string(),
            "login".to_string(),
            "subscription name".to_string(),
        ];
        assert_eq!(
            windows_terminal_args(&argv),
            vec![
                "/K",
                r"C:\Program Files\Codex\codex.exe",
                "login",
                "subscription name"
            ]
        );
    }

    #[test]
    fn auth_method_parses_terminal_command() {
        let raw = r#"{"id":"claude-ai-login","name":"Claude Subscription","description":"Use Claude subscription","type":"terminal","command":["claude","auth","login","--claudeai"]}"#;
        let method: AcpAuthMethod = serde_json::from_str(raw).unwrap();
        assert_eq!(method.method_type.as_deref(), Some("terminal"));
        assert_eq!(method.command[0], "claude");
    }

    #[test]
    fn claude_subscription_methods_run_without_a_visible_terminal() {
        for id in ["claude-login", "claude-ai-login"] {
            let method = AcpAuthMethod {
                id: id.into(),
                name: "Claude Subscription".into(),
                description: None,
                method_type: Some("terminal".into()),
                args: vec![],
                command: vec![],
                meta: None,
            };
            assert!(is_claude_subscription_login("claude", &method));
            assert!(!is_claude_subscription_login("codex", &method));
        }
    }

    #[test]
    fn other_claude_terminal_methods_remain_visible() {
        let method = AcpAuthMethod {
            id: "console-login".into(),
            name: "Anthropic Console".into(),
            description: None,
            method_type: Some("terminal".into()),
            args: vec![],
            command: vec![],
            meta: None,
        };
        assert!(!is_claude_subscription_login("claude", &method));
    }

    #[test]
    fn claude_terminal_meta_routes_around_unimplemented_authenticate() {
        let raw = r#"{"_meta":{"terminal-auth":{"args":["/tmp/claude-cli.js"],"command":"node","label":"Claude Login"}},"description":"Run `claude /login` in the terminal","id":"claude-login","name":"Log in with Claude"}"#;
        let method: AcpAuthMethod = serde_json::from_str(raw).unwrap();
        assert_eq!(method.method_type, None);
        assert!(uses_terminal_auth(&method).unwrap());
    }

    #[test]
    fn claude_terminal_meta_starts_login_flow() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let method: AcpAuthMethod = serde_json::from_str(
            r#"{"_meta":{"terminal-auth":{"args":["/tmp/claude-cli.js"],"command":"definitely-not-on-path-node"}},"id":"claude-login","name":"Log in with Claude"}"#,
        )
        .unwrap();
        assert_eq!(
            adapter_terminal_argv("Claude Code", &method, "unused").unwrap(),
            vec![
                "definitely-not-on-path-node".to_string(),
                "/tmp/claude-cli.js".to_string(),
                "auth".to_string(),
                "login".to_string(),
            ]
        );
    }

    #[test]
    fn terminal_argv_uses_adapter_declared_command() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let method = AcpAuthMethod {
            id: "claude-ai-login".into(),
            name: "Claude Subscription".into(),
            description: None,
            method_type: Some("terminal".into()),
            args: vec!["should-not".into(), "be-used".into()],
            command: vec![
                "definitely-not-on-path-buzz-test".into(),
                "auth".into(),
                "login".into(),
            ],
            meta: None,
        };
        assert_eq!(
            adapter_terminal_argv("Claude Code", &method, "claude-agent-acp").unwrap(),
            vec![
                "definitely-not-on-path-buzz-test".to_string(),
                "auth".to_string(),
                "login".to_string()
            ]
        );
    }

    #[test]
    fn terminal_argv_prefers_terminal_auth_meta_command() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let method = AcpAuthMethod {
            id: "claude-ai-login".into(),
            name: "Claude Subscription".into(),
            description: None,
            method_type: Some("terminal".into()),
            args: vec!["fallback-arg".into()],
            command: vec!["fallback-command".into()],
            meta: Some(serde_json::json!({
                "terminal-auth": {
                    "command": "definitely-not-on-path-meta",
                    "args": ["auth", "login", "--claudeai"]
                }
            })),
        };
        assert_eq!(
            adapter_terminal_argv("Claude Code", &method, "adapter-fallback").unwrap(),
            vec![
                "definitely-not-on-path-meta".to_string(),
                "auth".to_string(),
                "login".to_string(),
                "--claudeai".to_string()
            ]
        );
    }

    #[test]
    fn terminal_argv_falls_back_to_adapter_command() {
        let _guard = crate::managed_agents::lock_path_mutex();
        let method = AcpAuthMethod {
            id: "claude-ai-login".into(),
            name: "Claude Subscription".into(),
            description: None,
            method_type: Some("terminal".into()),
            args: vec![],
            command: vec![],
            meta: None,
        };
        assert_eq!(
            adapter_terminal_argv("Claude Code", &method, "definitely-not-on-path-buzz-test")
                .unwrap(),
            vec!["definitely-not-on-path-buzz-test".to_string()]
        );
    }
}
