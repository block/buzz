#![deny(unsafe_code)]
#![warn(missing_docs)]
//! # sprout-mcp
//!
//! MCP (Model Context Protocol) server that exposes [Sprout] — a Nostr-based enterprise
//! communications platform — as a set of tools consumable by AI agents.
//!
//! ## Overview
//!
//! `sprout-mcp` runs as a stdio MCP server. An agent host (e.g. Claude Desktop, Goose)
//! launches it as a subprocess and communicates over JSON-RPC on stdin/stdout. The server
//! maintains a persistent, authenticated WebSocket connection to a Sprout relay. All reads
//! use Nostr REQ/EOSE queries; all writes publish signed Nostr events.
//!
//! ```text
//!  ┌─────────────┐  JSON-RPC (stdio)  ┌──────────────┐  NIP-42 WebSocket  ┌───────────────┐
//!  │  Agent Host │ ◄─────────────────► │  sprout-mcp  │ ◄─────────────────► │ Sprout Relay  │
//!  └─────────────┘                     └──────────────┘  HTTP (media only)  └───────────────┘
//! ```
//!
//! ## Connecting to the Relay
//!
//! On startup `sprout-mcp` reads three environment variables:
//!
//! | Variable             | Default                  | Description                                      |
//! |----------------------|--------------------------|--------------------------------------------------|
//! | `SPROUT_RELAY_URL`   | `ws://localhost:3000`    | WebSocket URL of the Sprout relay                |
//! | `SPROUT_PRIVATE_KEY` | *(generated)*            | `nsec…` Nostr private key for the agent identity |
//! | `SPROUT_API_TOKEN`   | *(none)*                 | Auth token embedded in NIP-42 handshake          |
//!
//! If `SPROUT_PRIVATE_KEY` is absent a fresh ephemeral keypair is generated and its public key
//! is printed to stderr. In production you should supply a stable key so the agent has a
//! consistent Nostr identity.
//!
//! Authentication follows [NIP-42]: the relay sends an `AUTH` challenge immediately after the
//! WebSocket handshake; the client signs it and sends back an `AUTH` event. When
//! `SPROUT_API_TOKEN` is set the token is embedded in the auth event tags so the relay can
//! verify the agent's API permissions.
//!
//! ## WebSocket Connection Management
//!
//! [`relay_client::RelayClient`] uses a background tokio task that owns the WebSocket
//! connection. The background task:
//!
//! - Responds to Ping frames immediately — preventing relay disconnects during long LLM turns
//! - Handles mid-session NIP-42 AUTH challenges automatically
//! - Reconnects with exponential backoff (1 s → 2 s → 4 s → … → 30 s cap) on any
//!   connection loss, without any action required from the caller
//! - Re-authenticates via NIP-42 after each reconnect
//! - Replays all active subscriptions after reconnect
//!
//! ```text
//! RelayClient (Clone)
//!   ├── cmd_tx: mpsc::Sender<RelayCommand>   ← send_event / subscribe / close
//!   └── bg_handle: JoinHandle<()>
//!         └── run_background_task()
//!               ├── ws.next()  → handle_ws_message()   // Ping→Pong, AUTH→respond, OK→resolve
//!               ├── cmd_rx     → handle_command()       // SendEvent, Subscribe, Close
//!               └── tick       → expire_timed_out()     // 10s timeouts
//! ```
//!
//! ## Available Tools
//!
//! Tools are organized into toolsets; set `SPROUT_TOOLSETS` to control which are
//! active. The authoritative list and per-toolset grouping is `ALL_TOOLS` in
//! [`toolsets`] — this doc deliberately doesn't duplicate it, since a hand-copied
//! list drifts. The default toolset covers messaging, threads, search, feed,
//! reactions, channel basics, DMs, profiles/presence, and workflow triggers;
//! opt-in toolsets add channel admin, canvas, workflow admin, forums, social,
//! and media.
//!
//! ## Example Configuration (Claude Desktop)
//!
//! ```json
//! {
//!   "mcpServers": {
//!     "sprout": {
//!       "command": "/usr/local/bin/sprout-mcp-server",
//!       "env": {
//!         "SPROUT_RELAY_URL": "wss://relay.example.com",
//!         "SPROUT_PRIVATE_KEY": "nsec1...",
//!         "SPROUT_API_TOKEN": "your-api-token"
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! [Sprout]: https://github.com/block/sprout
//! [NIP-42]: https://github.com/nostr-protocol/nips/blob/master/42.md

// NOTE: `parse_relay_message`, `OkResponse`, and `RelayMessage` from `relay_client`
// are re-exported by `sprout-test-client`. Changes to these types are a breaking
// change for the test harness.

/// WebSocket client for the Sprout relay (NIP-42 auth, subscriptions, reconnect).
pub mod relay_client;
/// MCP tool implementations backed by the relay client.
pub mod server;
/// Toolset definitions and configuration for organizing MCP tools.
pub mod toolsets;
/// File upload to the Sprout relay (Blossom protocol).
pub mod upload;
