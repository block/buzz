//! Shared bounded source clients used by Command Adviser.

pub mod mcp_http;
pub mod oauth;
pub mod usage;
pub mod world_monitor;

pub const DEFAULT_WORLD_MONITOR_ENDPOINT: &str = "https://api.worldmonitor.app/mcp";
pub const WORLD_MONITOR_OAUTH_FILENAME: &str = "world-monitor-oauth.json";
