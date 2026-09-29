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
use std::sync::Arc;

mod browser_agent;
mod paths;
mod read_file;
mod rg;
mod shell;
mod shim;
mod str_replace;
mod term_session_card;
mod todo;
mod tree;
mod user_signer;
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
        name = "browser_observe_poll",
        description = "Poll Observe events for a Buzz in-app browser you hold Observe or Drive on. Console + network (headers/status/bodies) flow under Observe alone — Drive is not required. Event kinds include: grant, nav, console, network, snapshot, tab_opened, tab_switched, drive, drive_error, drive_started. Prefer surface_id (stable across popout/detach); webview_label also works. Omit both when you have exactly one grant. Pass after_id from the last event id to advance. Requires BUZZ_AGENT_PUBKEY. Returns JSON {grant, webviewLabel, surfaceId, webviewHidden, parked, driveContext, runbook?, events}. driveContext lists live surfaceId + browser_* tools. webviewHidden/parked when WKWebView is hide()d. Use browser_runbook_get / browser_runbook_propose (auto-activates unless persisted). Not OpenClaw Chromium."
    )]
    async fn browser_observe_poll(
        &self,
        Parameters(p): Parameters<browser_agent::ObservePollParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::observe_poll(p)
    }

    #[tool(
        name = "browser_agent_grants",
        description = "List Buzz browser Observe/Drive grants for this agent (BUZZ_AGENT_PUBKEY). Each grant includes surfaceId (preferred stable target) and webviewLabel (live host label). Browser groups may have multiple tabs — use browser_tabs / browser_switch_tab; main tab is primary focus. Returns JSON {grants}."
    )]
    async fn browser_agent_grants(
        &self,
        Parameters(p): Parameters<browser_agent::GrantsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let _ = p;
        browser_agent::grants(browser_agent::GrantsParams {})
    }

    #[tool(
        name = "browser_tabs",
        description = "List tabs in the Buzz in-app browser group for your Observe/Drive grant. Prefer surface_id. Returns mainTabSid (primary focus, no close), activeTabSid, and tabs[{surfaceId,url,title,isMain}]. Extra tabs open from in-page window.open/target=_blank only. Use browser_switch_tab to focus; poll browser_observe_poll for kind=tab_opened|tab_switched. Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_tabs(
        &self,
        Parameters(p): Parameters<browser_agent::TabsParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::tabs(p)
    }

    #[tool(
        name = "browser_switch_tab",
        description = "Focus a tab (by surface_id) in a Buzz browser group you hold Observe/Drive on. Main tab is primary focus; extras come from in-page new-window. Rebinds the grant onto the focused tab. Returns queued=true; poll browser_observe_poll for tab_switched or call browser_tabs. Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_switch_tab(
        &self,
        Parameters(p): Parameters<browser_agent::SwitchTabParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::switch_tab(p)
    }

    #[tool(
        name = "browser_get_viewport",
        description = "Read the Desktop Stage viewport for a Buzz browser you hold Observe or Drive on (mode desktop|responsive|mobile, width×height, mobile deviceId/orientation/scalePercent). Prefer surface_id. Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_get_viewport(
        &self,
        Parameters(p): Parameters<browser_agent::GetViewportParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::get_viewport(p)
    }

    #[tool(
        name = "browser_set_viewport",
        description = "Set Desktop Stage viewport while Driving (same controls humans see: mode desktop|responsive|mobile; responsive width×height; mobile deviceId + orientation portrait|landscape + scalePercent 50–200). Prefer surface_id. Drive grant required. Returns queued=true; Desktop applies via setPlaygroundViewport. Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_set_viewport(
        &self,
        Parameters(p): Parameters<browser_agent::SetViewportParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::set_viewport(p)
    }

    #[tool(
        name = "browser_drive",
        description = "Drive a Buzz WKWebView you hold in Drive mode. Prefer surface_id. Protocol: one goal/one surface; snapshot then one click/type/key; waitFor after nav/URL change; prefer browser_fill_field for forms; do not screenshot every step; on no element retry once then stop. `action`: { kind, id?, url?, x?, y?, text?, selector?, ref?, dx?, dy?, key?, urlContains?, timeoutMs?, clear? }. kind=navigate|click|type|fill|scroll|hover|key|waitFor. Optional include_snapshot=true returns inline snapshot with results. Returns {ok, results, url, ids, complete, elapsedMs, webviewLabel, surfaceId, snapshot?}. Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_drive(
        &self,
        Parameters(p): Parameters<browser_agent::DriveParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::drive(p)
    }

    #[tool(
        name = "browser_fill_field",
        description = "One-shot Drive fill: click/focus a field, type text, verify the value. Prefer over separate browser_drive click+type. Prefer surface_id. Target with selector, snapshot ref (e0), or x,y. clear defaults true. Optional include_snapshot. Requires Drive grant + BUZZ_AGENT_PUBKEY. Returns same shape as browser_drive including elapsedMs."
    )]
    async fn browser_fill_field(
        &self,
        Parameters(p): Parameters<browser_agent::FillFieldParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::fill_field(p)
    }

    #[tool(
        name = "browser_snapshot",
        description = "Request a DOM/a11y snapshot for a granted Buzz browser and wait for the inline payload (no stub+poll). Prefer surface_id. Returns {ok, snapshot, url, title, elapsedMs, grant, surfaceId}. Interactives include ref e0.. + center. Optional screenshot=true. Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_snapshot(
        &self,
        Parameters(p): Parameters<browser_agent::SnapshotParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::snapshot(p)
    }

    #[tool(
        name = "browser_runbook_get",
        description = "Read the site runbook for a Buzz browser you hold Observe or Drive on. Prefer surface_id. Without procedure_id returns agentBrief + active procedure index (id/title/summary). With procedure_id returns full markdown steps for that entry. Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_runbook_get(
        &self,
        Parameters(p): Parameters<browser_agent::RunbookGetParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::runbook_get(p)
    }

    #[tool(
        name = "browser_runbook_propose",
        description = "Propose/upsert a how-to procedure on a browser you hold Observe or Drive on. Prefer surface_id. Auto-activates agent-authored procedures. Rejected if a human-persisted procedure has the same title. Agent brief is human-owned (do not overwrite). Requires BUZZ_AGENT_PUBKEY."
    )]
    async fn browser_runbook_propose(
        &self,
        Parameters(p): Parameters<browser_agent::RunbookProposeParams>,
    ) -> Result<CallToolResult, ErrorData> {
        browser_agent::runbook_propose(p)
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
        name = "buzz_read_thread",
        description = "Read a Buzz thread as the signed-in Desktop user via IPC signer (no nsec in this process). Requires BUZZ_USER_SIGNER_DIR from Term launch. Returns JSON {ok, events[], asUser}."
    )]
    async fn buzz_read_thread(
        &self,
        Parameters(p): Parameters<user_signer::ReadThreadParams>,
    ) -> Result<CallToolResult, ErrorData> {
        user_signer::read_thread(p)
    }

    #[tool(
        name = "buzz_read_channel",
        description = "Read recent top-level Buzz channel messages as the signed-in Desktop user via IPC signer. Requires BUZZ_USER_SIGNER_DIR. Returns JSON {ok, events[], asUser}."
    )]
    async fn buzz_read_channel(
        &self,
        Parameters(p): Parameters<user_signer::ReadChannelParams>,
    ) -> Result<CallToolResult, ErrorData> {
        user_signer::read_channel(p)
    }

    #[tool(
        name = "buzz_draft_message",
        description = "Prepare a Desktop composer draft of content the **human asked you to write for them to send** (draft-only — never auto-publishes; JM clicks Send). Do NOT use for agent progress/status, acknowledgements, or chatting with the user — those belong in the Term TUI. Pass channel_id, content, optional thread_id, and optional mentions: [{ displayName, pubkey, isAgent }] (camelCase) so Desktop creates real @mention chips — plain @Name in content alone is NOT enough. Requires BUZZ_USER_SIGNER_DIR."
    )]
    async fn buzz_draft_message(
        &self,
        Parameters(p): Parameters<user_signer::DraftMessageParams>,
    ) -> Result<CallToolResult, ErrorData> {
        user_signer::draft_message(p)
    }

    #[tool(
        name = "term_session_card",
        description = "Standing instructions: summarize the thread into a Buzz Term handoff. Reply in chat with ONLY a short one-line ack plus this tool’s returned fenced card. Put the full handoff prompt ONLY in JSON `prompt` (UI hides it). Never dump the prompt as plain markdown. Never put tokens/JWTs in the card. Set `openclawWorkspace` true (boolean only) when the agent uses OpenClaw; otherwise omit or false. Pass originChannelId + originThreadId + mentionToUse + mentionPubkey (hex pubkey of the Buzz agent to @ on hand-back) so the tool appends a Return path (Buzz) section: Term must buzz_draft_message to that origin with content `@DisplayName …` AND mentions: [{ displayName, pubkey, isAgent: true }] — plain @Name alone is not enough — on report back / hand back / I'm done; never to a summarized/source thread."
    )]
    async fn term_session_card(
        &self,
        Parameters(p): Parameters<term_session_card::TermSessionCardParams>,
    ) -> Result<String, ErrorData> {
        term_session_card::run(p)
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
