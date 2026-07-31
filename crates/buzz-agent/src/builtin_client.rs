//! Buzz's in-process tools, exposed to Goose as a platform extension.
//!
//! Goose has two ways to take a tool the embedder implements in Rust:
//!
//! * [`ExtensionConfig::Frontend`] — Goose *advertises* the tool but refuses to
//!   run it. It returns "Frontend tool execution required" and blocks until the
//!   embedder calls `handle_tool_result`. Dispatch is strictly sequential, has
//!   no timeout, and the result channel carries no request id, so a single
//!   missed result wedges the agent for the rest of the session and the cancel
//!   token will not free it.
//! * [`ExtensionConfig::Platform`] + `extension_manager.add_client` with an
//!   [`McpClientTrait`] — Goose dispatches it through the ordinary tool path,
//!   so concurrency, per-request timeouts and `notifications/cancelled` all
//!   work, and there is nothing for us to correlate.
//!
//! This uses the second. It is the shape Maple settled on for its own
//! in-process developer and skills tools.
//!
//! The only tool here is `load_skill`. The system prompt carries the skill
//! *index* (names and descriptions, from [`crate::hints`]); bodies are pulled
//! on demand so a large skill library costs nothing per turn.

use std::sync::Arc;

use async_trait::async_trait;
use goose::agents::mcp_client::McpClientTrait;
use goose::agents::{ExtensionConfig, ToolCallContext};
use rmcp::model::{CallToolResult, Content, InitializeResult, JsonObject, ListToolsResult, Tool};
use rmcp::service::ServiceError as Error;
use tokio_util::sync::CancellationToken;

use crate::hints::SkillEntry;

/// Extension key and display name. Goose prefixes tool names with the
/// extension name (`{extension}__{tool}`) unless the extension is registered
/// as unprefixed, so this shows up in the model's tool list.
pub const BUILTIN_EXTENSION_NAME: &str = "buzz";

/// Goose-side declaration for the extension backed by [`BuiltinClient`].
pub fn builtin_extension_config() -> ExtensionConfig {
    ExtensionConfig::Platform {
        name: BUILTIN_EXTENSION_NAME.to_string(),
        description: "Buzz built-in tools".to_string(),
        display_name: Some("Buzz".to_string()),
        bundled: Some(true),
        available_tools: vec![crate::builtin::LOAD_SKILL_TOOL.to_string()],
    }
}

/// In-process MCP client serving [`crate::builtin`]'s tools.
pub struct BuiltinClient {
    skills: Vec<SkillEntry>,
}

impl BuiltinClient {
    pub fn new(skills: Vec<SkillEntry>) -> Self {
        Self { skills }
    }

    fn load_skill_tool() -> Tool {
        let def = crate::builtin::load_skill_def();
        let schema = match def.input_schema {
            serde_json::Value::Object(map) => Arc::new(map),
            _ => Arc::new(serde_json::Map::new()),
        };
        Tool::new(def.name, def.description, schema)
    }
}

#[async_trait]
impl McpClientTrait for BuiltinClient {
    async fn list_tools(
        &self,
        _session_id: &str,
        _next_cursor: Option<String>,
        _cancel_token: CancellationToken,
    ) -> Result<ListToolsResult, Error> {
        Ok(ListToolsResult {
            tools: vec![Self::load_skill_tool()],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        _ctx: &ToolCallContext,
        name: &str,
        arguments: Option<JsonObject>,
        _cancel_token: CancellationToken,
    ) -> Result<CallToolResult, Error> {
        if name != crate::builtin::LOAD_SKILL_TOOL {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "unknown tool: {name}"
            ))]));
        }

        let args = arguments
            .map(serde_json::Value::Object)
            .unwrap_or(serde_json::Value::Null);
        let result = crate::builtin::call_load_skill(&args, &self.skills).await;

        let content: Vec<Content> = result
            .content
            .iter()
            .map(|c| Content::text(c.as_text_lossy()))
            .collect();

        Ok(if result.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        })
    }

    fn get_info(&self) -> Option<&InitializeResult> {
        None
    }
}
