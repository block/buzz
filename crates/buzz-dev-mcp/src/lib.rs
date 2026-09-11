#![cfg_attr(not(windows), forbid(unsafe_code))]
#![cfg_attr(windows, deny(unsafe_code))]
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData, ServerHandler, ServiceExt,
};
use std::path::Path;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::sync::Arc;
#[cfg(target_os = "macos")]
use zeroize::Zeroize;

mod paths;
mod read_file;
mod rg;
mod shell;
mod shim;
mod str_replace;
mod todo;
mod tree;
mod view_image;

#[derive(Clone)]
struct DevMcp {
    state: Arc<shell::SharedState>,
    todos: Arc<todo::TodoState>,
    tool_router: ToolRouter<DevMcp>,
}

#[tool_router]
impl DevMcp {
    fn new(state: Arc<shell::SharedState>) -> Self {
        Self {
            state,
            todos: Arc::new(todo::TodoState::new()),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        name = "shell",
        description = "Run a shell command (bash by default; set `BUZZ_SHELL` to use cmd, PowerShell, or another shell). Ephemeral process per call. Output tail-truncated to ~8KB for the LLM; full output (first 10MB) saved to artifact file. timeout_ms defaults to 120000 (2 min) if omitted; capped at 1,200,000 (20 min). For long-running commands (git push with hooks, cargo build, test suites), use 300000+. On PATH: rg (prefer over grep; flags: -n -i -l -g <glob> -C <n> --files), tree (flags: -d <depth>; shows line counts), and buzz (Buzz relay CLI — run buzz --help for commands)."
    )]
    async fn shell(
        &self,
        Parameters(p): Parameters<shell::ShellParams>,
        context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        shell::run(&self.state, p, context.ct).await
    }

    #[tool(
        name = "read_file",
        description = "Read a text file and return its contents with line numbers. Returns lines in `{number}:{content}` format. Use `offset` (0-based) and `limit` (default 2000) to window into large files. Path resolved relative to workdir (defaults to server cwd). Prefer over cat/head/tail."
    )]
    async fn read_file(
        &self,
        Parameters(p): Parameters<read_file::ReadFileParams>,
    ) -> Result<String, ErrorData> {
        read_file::run(&self.state, p)
    }

    #[tool(
        name = "view_image",
        description = "Load an image from a file path, http(s) URL, or data: URL and return it as an MCP image content block that multimodal LLMs (Anthropic, OpenAI-compatible, etc.) can see. Resizes to a longest-edge of 1568px by default (override with `max_dim`, range 64..=2048). Pass-through for already-small PNG/JPEG; transcodes oversize input to PNG (if alpha) or JPEG q85. Animated GIF/WebP rejected — provide a still frame. Hard cap 20 MiB source, ~4 MiB on the wire. Relative paths resolve under `workdir` (defaults to server cwd) and may not escape it."
    )]
    async fn view_image(
        &self,
        Parameters(p): Parameters<view_image::ViewImageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        view_image::run(&self.state, p).await
    }

    #[tool(
        name = "str_replace",
        description = "Atomic find-and-replace in a file. old_str must occur exactly once unless replace_all is true, in which case all occurrences are replaced. Returns a unified diff. Path resolved relative to workdir (defaults to server cwd). Prefer over sed/awk."
    )]
    async fn str_replace(
        &self,
        Parameters(p): Parameters<str_replace::StrReplaceParams>,
    ) -> Result<String, ErrorData> {
        str_replace::run(&self.state, p)
    }

    #[tool(
        name = "todo",
        description = "Session checklist only for work that must continue across turns or survive context compaction. Do not use for work you can finish in the current turn. Omit `todos` to read; provide the full {text, done} list to replace it. Open items let the _Stop hook advise against ending."
    )]
    async fn todo(
        &self,
        Parameters(p): Parameters<todo::TodoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        match self.todos.handle_todo(p) {
            Ok(text) => todo::text_result(text),
            Err(e) => todo::error_result(format!("Error: {e}")),
        }
    }

    /// Hook: called by the agent before honoring end_turn. Returns
    /// non-empty objection text iff items remain open.
    #[tool(
        name = "_Stop",
        description = "Returns open todo items if any exist. Used by the agent's _Stop lifecycle hook to advise against ending with incomplete work."
    )]
    async fn stop_hook(
        &self,
        Parameters(_): Parameters<todo::HookParams>,
    ) -> Result<CallToolResult, ErrorData> {
        todo::text_result(self.todos.stop_objection())
    }

    /// Hook: called by the agent after context compaction/handoff so the
    /// todo list survives history truncation.
    #[tool(
        name = "_PostCompact",
        description = "Internal hook. Agent invokes after handoff; returns todo state for re-injection."
    )]
    async fn post_compact_hook(
        &self,
        Parameters(_): Parameters<todo::HookParams>,
    ) -> Result<CallToolResult, ErrorData> {
        todo::text_result(self.todos.post_compact())
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for DevMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "buzz-dev-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.state.bootstrap_instructions.clone())
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let argv0 = std::env::args().next().unwrap_or_default();
    let cmd = Path::new(&argv0)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Multicall dispatch — sync personalities exit before any runtime is built.
    // No tracing, no tokio, no allocations beyond argv parsing.
    match cmd.as_str() {
        "rg" => std::process::exit(rg::run(std::env::args().skip(1).collect())),
        "tree" => std::process::exit(tree::run(std::env::args().skip(1).collect())),
        "git-credential-nostr" => std::process::exit(git_credential_nostr::run()),
        "git-sign-nostr" => std::process::exit(git_sign_nostr::run()),
        _ => {}
    }

    // Async personalities and MCP server mode — build the runtime.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async_main(cmd))
}

async fn async_main(cmd: String) -> Result<(), Box<dyn std::error::Error>> {
    // HTTPS clients invoked through this MCP process need a Rustls provider;
    // repeated installation is harmless.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // buzz CLI needs tokio (async HTTP client).
    if cmd == "buzz" {
        std::process::exit(buzz_cli::run_from_args(std::env::args()).await);
    }

    load_keychain_private_key()?;

    // MCP server mode — safe to init tracing now.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let cwd = std::env::current_dir()?;
    let shim = shim::Shim::install()?;
    let state = Arc::new(shell::SharedState::new(cwd, shim)?);

    let service = DevMcp::new(state).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// Resolve a macOS Keychain-backed Buzz identity for this MCP process.
///
/// A configured service is authoritative: lookup or validation failure aborts
/// startup instead of falling back to an inherited credential.
#[cfg(target_os = "macos")]
fn load_keychain_private_key() -> Result<(), Box<dyn std::error::Error>> {
    let Some(service) = keychain_service_from_env(std::env::var("BUZZ_KEYCHAIN_SERVICE"))? else {
        return Ok(());
    };
    std::env::remove_var("BUZZ_KEYCHAIN_SERVICE");
    let account = std::env::var("USER")
        .ok()
        .filter(|account| !account.is_empty())
        .ok_or("buzz-dev-mcp: USER is unavailable for Keychain lookup")?;

    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-a",
            &account,
            "-s",
            &service,
            "-w",
        ])
        .output()
        .map_err(|error| format!("buzz-dev-mcp: failed to run Keychain lookup: {error}"))?;
    if !output.status.success() {
        return Err("buzz-dev-mcp: Keychain service is unavailable".into());
    }

    let mut stored = String::from_utf8(output.stdout)
        .map_err(|_| "buzz-dev-mcp: Keychain service returned non-UTF-8 data")?;
    while stored.ends_with(['\r', '\n']) {
        stored.pop();
    }
    let result = (|| {
        let private_key = keychain_private_key_value(&stored);
        if private_key.is_empty() || nostr::Keys::parse(private_key).is_err() {
            return Err("buzz-dev-mcp: Keychain service returned an invalid private key".into());
        }
        std::env::set_var("BUZZ_PRIVATE_KEY", private_key);
        Ok(())
    })();
    stored.zeroize();
    result
}

#[cfg(target_os = "macos")]
fn keychain_service_from_env(
    value: Result<String, std::env::VarError>,
) -> Result<Option<String>, &'static str> {
    match value {
        Ok(service) if service.is_empty() => {
            Err("buzz-dev-mcp: BUZZ_KEYCHAIN_SERVICE must not be empty")
        }
        Ok(service) => Ok(Some(service)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err("buzz-dev-mcp: BUZZ_KEYCHAIN_SERVICE must be valid UTF-8")
        }
    }
}

/// Preserve a raw key while accepting the legacy `pubkey:private-key` value.
#[cfg(target_os = "macos")]
fn keychain_private_key_value(value: &str) -> &str {
    value
        .split_once(':')
        .map(|(_, private_key)| private_key)
        .unwrap_or(value)
}

#[cfg(not(target_os = "macos"))]
fn load_keychain_private_key() -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
mod keychain_tests {
    use super::{keychain_private_key_value, keychain_service_from_env};

    #[test]
    fn rejects_empty_keychain_service() {
        let error = keychain_service_from_env(Ok(String::new())).unwrap_err();
        assert_eq!(
            error,
            "buzz-dev-mcp: BUZZ_KEYCHAIN_SERVICE must not be empty"
        );
    }

    #[test]
    fn preserves_raw_private_key() {
        assert_eq!(keychain_private_key_value("nsec1rawtest"), "nsec1rawtest");
    }

    #[test]
    fn strips_legacy_prefix() {
        assert_eq!(
            keychain_private_key_value("pubkey:nsec1prefixedtest"),
            "nsec1prefixedtest"
        );
    }
}

/// Suppress the console window that Windows otherwise allocates for every
/// console-subsystem child process spawned from a non-console parent.
/// No-op on non-Windows platforms.
pub(crate) fn configure_no_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Suppress the console window for async (`tokio::process::Command`) spawns.
/// Equivalent to `configure_no_window` but accepts a tokio command.
/// No-op on non-Windows platforms.
pub(crate) fn configure_no_window_async(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}
