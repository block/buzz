//! Optional per-turn observation hook for product-specific ACP integrations.
//!
//! The ACP client owns protocol transport. Integrations may observe normalized
//! `session/update` payloads, but must keep product policy and result parsing
//! outside the transport implementation.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use nostr::Event;
use uuid::Uuid;

use crate::acp::{AcpClient, McpServer};
use crate::relay::RestClient;

pub(crate) trait TurnObserver: Send {
    fn on_session_update(&mut self, update: &serde_json::Value);

    fn into_any(self: Box<Self>) -> Box<dyn Any + Send>;
}

pub(crate) type TurnExtensionFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Clone, Copy)]
pub(crate) struct TurnExtensionStartupPolicy {
    pub(crate) replace_standard_mcp_servers: bool,
    pub(crate) max_turn_duration: Option<Duration>,
    pub(crate) base_prompt: Option<&'static str>,
    pub(crate) disable_memory: bool,
}

pub(crate) struct TurnExtensionRequest<'a> {
    pub(crate) source_event: Option<&'a Event>,
    pub(crate) channel_id: Option<Uuid>,
    pub(crate) agent_id: &'a str,
    pub(crate) turn_id: &'a str,
}

pub(crate) struct TurnExtensionFinishContext<'a> {
    pub(crate) acp: &'a mut AcpClient,
    pub(crate) rest_client: &'a RestClient,
    pub(crate) source_event: Option<&'a Event>,
    pub(crate) channel_id: Option<Uuid>,
    pub(crate) completed: bool,
}

/// Product integration applied to one ACP turn.
///
/// Implementations may inject an ephemeral MCP server and observe the turn,
/// but the pool remains unaware of product policy and credentials.
pub(crate) trait TurnExtension: Send + Sync {
    fn startup_policy(&self) -> TurnExtensionStartupPolicy;

    fn begin_turn<'a>(
        &'a self,
        request: TurnExtensionRequest<'a>,
    ) -> TurnExtensionFuture<'a, Result<Option<Box<dyn TurnExtensionAccess>>, String>>;
}

/// Per-turn state returned by a [`TurnExtension`]. Dropping it must release
/// any capability that should not survive the turn.
pub(crate) trait TurnExtensionAccess: Send {
    fn mcp_server(&self) -> Option<&McpServer>;

    fn requires_fresh_session(&self) -> bool;

    fn start_observation(&mut self, acp: &mut AcpClient);

    fn finish<'a>(
        &'a mut self,
        context: TurnExtensionFinishContext<'a>,
    ) -> TurnExtensionFuture<'a, ()>;
}
