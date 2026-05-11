#![forbid(unsafe_code)]
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData, ServerHandler, ServiceExt,
};
use std::path::Path;
use std::sync::Arc;

mod rg;
mod shell;
mod shim;
mod str_replace;
mod todo;
mod tree;

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
        description = "Run a bash command. Ephemeral process per call. Output tail-truncated to ~8KB for the LLM; full output (first 10MB) saved to artifact file. timeout_ms capped at 600000. On PATH: rg (prefer over grep; flags: -n -i -l -g <glob> -C <n> --files), tree (flags: -d <depth>; shows line counts), and sprout (Sprout relay CLI — run sprout --help for commands)."
    )]
    async fn shell(
        &self,
        Parameters(p): Parameters<shell::ShellParams>,
        context: rmcp::service::RequestContext<rmcp::service::RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        shell::run(&self.state, p, context.ct).await
    }

    #[tool(
        name = "str_replace",
        description = "Atomic find-and-replace in a file. old_str must occur exactly once. Returns a unified diff. Path resolved relative to workdir (defaults to server cwd). Prefer over sed/awk."
    )]
    async fn str_replace(
        &self,
        Parameters(p): Parameters<str_replace::StrReplaceParams>,
    ) -> Result<String, ErrorData> {
        str_replace::run(&self.state, p)
    }

    #[tool(
        name = "todo",
        description = "Session task list. Omit `todos` to read current state. Provide a full replacement array to update. Items are {text, done}. Open items removed without being marked done will trigger a warning. If the operator enables hooks for this server, the agent's _Stop hook will advise against ending the turn while items are open."
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

#[tool_handler]
impl ServerHandler for DevMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "sprout-dev-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(self.state.bootstrap_instructions.clone())
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    let argv0 = std::env::args().next().unwrap_or_default();
    let cmd = Path::new(&argv0)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if cmd == "rg" {
        let args: Vec<String> = std::env::args().skip(1).collect();
        std::process::exit(rg::run(args));
    }

    if cmd == "tree" {
        let args: Vec<String> = std::env::args().skip(1).collect();
        std::process::exit(tree::run(args));
    }

    if cmd == "sprout" {
        std::process::exit(sprout_cli::run_from_args(std::env::args()).await);
    }

    let cwd = std::env::current_dir()?;
    let shim = shim::Shim::install()?;
    let state = Arc::new(shell::SharedState::new(cwd, shim)?);

    let service = DevMcp::new(state).serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
