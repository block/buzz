//! Local version-1 task input and terminal CLI contract. No service startup.
use crate::{
    config::{CliArgs, Config},
    isolated_execution::{self, Outcome},
    runtime::{AgentRuntime, SessionMode},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{io::Write, time::Duration};
use tokio::io::AsyncReadExt;

const MAX_TASK_BYTES: u64 = 1024 * 1024;
const INPUT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Parser)]
#[command(
    name = "buzz-acp run",
    about = "Run one local task in a fresh agent session, then exit",
    after_help = "--task accepts a local file or - (stdin). URLs are not supported.\nExisting launch options are accepted; service-only options do not start service features.\nstdout: one version-1 JSON terminal record; stderr: diagnostics.\nExit: 0 completed, 1 failed/cancelled, 2 invalid input/config, 124 deadline, 130 SIGINT, 143 SIGTERM."
)]
struct RunArgs {
    /// One versioned JSON task from a local file, or - for stdin.
    #[arg(long, value_name = "PATH|-")]
    task: String,
    #[command(flatten)]
    launch: CliArgs,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Task {
    version: u8,
    task_id: String,
    agent_pubkey: String,
    prompt: String,
    max_duration_ms: u64,
}

impl Task {
    fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let task: Self = serde_json::from_slice(bytes).map_err(|_| "invalid_task_json")?;
        if task.version != 1 {
            return Err("unsupported_task_version");
        }
        if task.task_id.is_empty()
            || task.task_id.len() > 256
            || task.task_id.chars().any(char::is_control)
            || task.task_id.trim().is_empty()
            || task.prompt.trim().is_empty()
            || task.max_duration_ms == 0
            || task.agent_pubkey.len() != 64
            || !task
                .agent_pubkey
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("invalid_task_fields");
        }
        Ok(task)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Terminal {
    version: u8,
    task_id: Option<String>,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}
impl Terminal {
    fn new(status: &'static str, error: Option<&'static str>) -> Self {
        Self {
            version: 1,
            task_id: None,
            status,
            error,
            stop_reason: None,
        }
    }
}

async fn read_task(source: &str) -> Result<Task, &'static str> {
    // Reserve URL-shaped sources for a future explicit retrieval contract.
    if source.contains("://") {
        return Err("remote_task_not_supported");
    }
    let mut bytes = Vec::new();
    let read = async {
        if source == "-" {
            tokio::io::stdin()
                .take(MAX_TASK_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
        } else {
            tokio::fs::File::open(source)
                .await?
                .take(MAX_TASK_BYTES + 1)
                .read_to_end(&mut bytes)
                .await
        }
    };
    tokio::time::timeout(INPUT_TIMEOUT, read)
        .await
        .map_err(|_| "task_input_timeout")?
        .map_err(|_| "task_read_failed")?;
    if bytes.len() as u64 > MAX_TASK_BYTES {
        return Err("task_too_large");
    }
    Task::parse(&bytes)
}

pub(crate) async fn run() -> i32 {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "buzz_acp=info".into()),
        )
        .compact()
        .try_init();
    let args = RunArgs::try_parse_from(std::env::args_os().skip(1));
    let mut args = match args {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return 0;
        }
        Err(_) => return emit(Terminal::new("invalid", Some("invalid_arguments")), 2),
    };
    // These settings belong only to conversation admission and repeated work.
    args.launch.heartbeat_interval = 0;
    args.launch.heartbeat_prompt = None;
    args.launch.heartbeat_prompt_file = None;
    args.launch.turn_liveness_secs = 0;
    let (signal_tx, mut signal_rx) = tokio::sync::oneshot::channel();
    // Register before configuration: launch prompt files can also block on I/O.
    #[cfg(unix)]
    let signals = (
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()),
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()),
    );
    #[cfg(unix)]
    let signal_task = match signals {
        (Ok(mut interrupt), Ok(mut terminate)) => tokio::spawn(async move {
            let code = tokio::select! { _ = interrupt.recv() => 130, _ = terminate.recv() => 143 };
            let _ = signal_tx.send(code);
        }),
        _ => return emit(Terminal::new("failed", Some("signal_setup_failed")), 1),
    };
    #[cfg(not(unix))]
    let signal_task = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = signal_tx.send(130);
    });
    // Config owns synchronous prompt-file reads. Keep them off the async worker
    // so signals and the preparation bound remain live, including for FIFOs.
    // This worker only builds configuration; it cannot spawn an agent or create
    // runtime resources. On cancellation/timeout the CLI's bounded runtime
    // shutdown and process exit also retire a still-blocked read.
    let configuration = tokio::task::spawn_blocking(move || Config::from_args(args.launch));
    let config = tokio::select! {
        biased;
        code = &mut signal_rx => return emit(Terminal::new("cancelled", None), code.unwrap_or(1)),
        result = tokio::time::timeout(INPUT_TIMEOUT, configuration) => match result {
            Ok(Ok(Ok(config))) if config.max_turn_duration_secs > 0 => config,
            Err(_) => {
                signal_task.abort();
                return emit(Terminal::new("invalid", Some("configuration_timeout")), 2);
            }
            _ => {
                signal_task.abort();
                return emit(Terminal::new("invalid", Some("invalid_configuration")), 2);
            }
        },
    };
    let task = tokio::select! {
        biased;
        code = &mut signal_rx => return emit(Terminal::new("cancelled", None), code.unwrap_or(1)),
        task = read_task(&args.task) => task,
    };
    let task = match task {
        Ok(task) => task,
        Err(error) => {
            signal_task.abort();
            return emit(Terminal::new("invalid", Some(error)), 2);
        }
    };
    let mut terminal = Terminal::new("invalid", None);
    terminal.task_id = Some(task.task_id);
    if task.agent_pubkey != config.keys.public_key().to_hex() {
        terminal.error = Some("agent_identity_mismatch");
        signal_task.abort();
        return emit(terminal, 2);
    }
    // Keep signing material alive until execution has drained and reaped the adapter.
    let runtime = match AgentRuntime::prepare(config) {
        Ok(environment) => environment,
        Err(_) => {
            terminal.status = "failed";
            terminal.error = Some("runtime_setup_failed");
            signal_task.abort();
            return emit(terminal, 1);
        }
    };
    let config = runtime.config();
    let rest = crate::relay::RestClient {
        http: reqwest::Client::new(),
        base_url: crate::relay::relay_ws_to_http(&config.relay_url),
        keys: config.keys.clone(),
        auth_tag_json: std::env::var("BUZZ_AUTH_TAG")
            .ok()
            .filter(|s| !s.is_empty()),
    };
    let ctx = match runtime.prompt_context(rest, Default::default(), SessionMode::Task) {
        Ok(ctx) => ctx,
        Err(_) => {
            terminal.error = Some("invalid_working_directory");
            return emit(terminal, 2);
        }
    };
    let startup = runtime.startup(None);
    let duration = Duration::from_millis(
        task.max_duration_ms
            .min(config.max_turn_duration_secs.saturating_mul(1000)),
    );
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let execution = isolated_execution::execute(&startup, &ctx, &task.prompt, duration, cancel_rx);
    tokio::pin!(execution);
    let (outcome, signal) = tokio::select! {
        biased;
        code = &mut signal_rx => {
            let _ = cancel_tx.send(());
            (execution.await, Some(code.unwrap_or(1)))
        }
        outcome = &mut execution => (outcome, None),
    };
    signal_task.abort();
    let code = if let Some(code) = signal {
        terminal.status = "cancelled";
        code
    } else {
        match outcome {
            Outcome::Completed(reason) => {
                terminal.status = "completed";
                terminal.stop_reason = Some(reason);
                0
            }
            Outcome::Cancelled => {
                terminal.status = "cancelled";
                1
            }
            Outcome::Deadline => {
                terminal.status = "timed_out";
                124
            }
            Outcome::Failed(error) => {
                terminal.status = "failed";
                terminal.error = Some(error);
                1
            }
        }
    };
    emit(terminal, code)
}

fn emit(terminal: Terminal, code: i32) -> i32 {
    let mut out = std::io::stdout().lock();
    if serde_json::to_writer(&mut out, &terminal).is_err()
        || out.write_all(b"\n").is_err()
        || out.flush().is_err()
    {
        return 1;
    }
    code
}
