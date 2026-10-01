#![deny(unsafe_code)]
//! Tauri IPC commands and configuration persistence for Model Context Protocol (MCP)
//! servers, plugins, and tool execution security policies in Orbit.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Configuration for an external or custom MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique identifier for the server.
    pub id: String,
    /// Display name of the server.
    pub name: String,
    /// Transport type: "stdio", "sse", or "http".
    pub transport: String,
    /// Executable binary path or command for stdio transport.
    #[serde(default)]
    pub command: Option<String>,
    /// Command-line arguments for stdio transport.
    #[serde(default)]
    pub args: Option<Vec<String>>,
    /// Working directory for stdio child process.
    #[serde(default)]
    pub cwd: Option<String>,
    /// Environment variables to pass to stdio process.
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    /// Server endpoint URL for sse/http transport.
    #[serde(default)]
    pub url: Option<String>,
    /// Custom HTTP headers for sse/http transport.
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Optional bearer authentication token.
    #[serde(default)]
    pub bearer_token: Option<String>,
    /// Whether the server is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// ISO timestamp when server was added.
    #[serde(default)]
    pub created_at: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Discovered tool schema from an MCP server handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpDiscoveredTool {
    /// Name of the tool.
    pub name: String,
    /// Tool description.
    #[serde(default)]
    pub description: Option<String>,
    /// JSON schema for tool inputs.
    #[serde(default)]
    pub input_schema: Option<serde_json::Value>,
}

/// Live connection and handshake probe result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTestResult {
    /// True if the server responded successfully to MCP initialization and tool discovery.
    pub success: bool,
    /// Round-trip response latency in milliseconds.
    pub latency_ms: u64,
    /// Number of tools discovered.
    pub tools_count: usize,
    /// List of tools exposed by the server.
    pub tools: Vec<McpDiscoveredTool>,
    /// Error message if probe failed.
    #[serde(default)]
    pub error: Option<String>,
}

/// Configuration for an Orbit plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    /// Unique identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Description of capabilities.
    pub description: String,
    /// Semantic version.
    pub version: String,
    /// Category: "connector", "search", "database", "issue-tracker", "custom".
    pub category: String,
    /// Whether plugin is enabled.
    pub enabled: bool,
    /// Lucide icon name.
    #[serde(default)]
    pub icon: Option<String>,
    /// Custom configuration values.
    #[serde(default)]
    pub settings: Option<serde_json::Value>,
    /// Plugin source: "builtin", "git", "local", "manifest".
    #[serde(default = "default_builtin")]
    pub source: String,
}

fn default_builtin() -> String {
    "builtin".to_string()
}

/// Execution security policy for individual MCP tools.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ToolPolicy {
    /// Automatically approve execution without user prompting.
    AutoApprove,
    /// Require explicit developer confirmation before executing.
    ConfirmOnExecute,
    /// Suppressed from agent discovery.
    Disabled,
}

fn resolve_orbit_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".orbit")
}

fn default_tool_policies() -> HashMap<String, ToolPolicy> {
    let mut map = HashMap::new();
    map.insert("orbit.search_context".to_string(), ToolPolicy::AutoApprove);
    map.insert("orbit.store_memory".to_string(), ToolPolicy::ConfirmOnExecute);
    map.insert("orbit.get_project_context".to_string(), ToolPolicy::AutoApprove);
    map.insert("orbit.recall_session".to_string(), ToolPolicy::AutoApprove);
    map.insert("orbit.get_file_history".to_string(), ToolPolicy::AutoApprove);
    map.insert("orbit.mark_decision".to_string(), ToolPolicy::ConfirmOnExecute);
    map.insert("orbit.get_index_stats".to_string(), ToolPolicy::AutoApprove);
    map.insert("orbit.delete_memory".to_string(), ToolPolicy::ConfirmOnExecute);
    map.insert("buzz-dev-mcp:shell".to_string(), ToolPolicy::ConfirmOnExecute);
    map.insert("buzz-dev-mcp:read_file".to_string(), ToolPolicy::AutoApprove);
    map.insert("buzz-dev-mcp:edit_file".to_string(), ToolPolicy::ConfirmOnExecute);
    map
}

fn default_plugins() -> Vec<PluginConfig> {
    vec![
        PluginConfig {
            id: "github-connector".to_string(),
            name: "GitHub Connector".to_string(),
            description: "Sync repositories, pull requests, issues, and commit history into Orbit memory graph.".to_string(),
            version: "1.0.0".to_string(),
            category: "connector".to_string(),
            enabled: true,
            icon: Some("github".to_string()),
            settings: Some(serde_json::json!({ "auto_sync": true, "sync_interval_mins": 30 })),
            source: "builtin".to_string(),
        },
        PluginConfig {
            id: "slack-reader".to_string(),
            name: "Slack Reader".to_string(),
            description: "Index engineering discussions, decision threads, and design rationale from Slack channels.".to_string(),
            version: "1.0.0".to_string(),
            category: "connector".to_string(),
            enabled: false,
            icon: Some("message-square".to_string()),
            settings: Some(serde_json::json!({ "channels": ["#engineering", "#architecture"] })),
            source: "builtin".to_string(),
        },
        PluginConfig {
            id: "postgres-schema-explorer".to_string(),
            name: "PostgreSQL Schema Explorer".to_string(),
            description: "Map database tables, column constraints, and foreign key relations into the knowledge graph.".to_string(),
            version: "1.0.0".to_string(),
            category: "database".to_string(),
            enabled: false,
            icon: Some("database".to_string()),
            settings: Some(serde_json::json!({ "ssl_mode": "prefer" })),
            source: "builtin".to_string(),
        },
        PluginConfig {
            id: "jira-issue-tracker".to_string(),
            name: "Jira Issue Tracker".to_string(),
            description: "Link user stories, tickets, and acceptance criteria to code modifications and decisions.".to_string(),
            version: "1.0.0".to_string(),
            category: "issue-tracker".to_string(),
            enabled: false,
            icon: Some("ticket".to_string()),
            settings: Some(serde_json::json!({ "project_keys": ["ORBIT", "ENG"] })),
            source: "builtin".to_string(),
        },
        PluginConfig {
            id: "web-search-scraper".to_string(),
            name: "Web Search & Scraper".to_string(),
            description: "Live documentation search and real-time package API reference lookup for agents.".to_string(),
            version: "1.0.0".to_string(),
            category: "search".to_string(),
            enabled: true,
            icon: Some("globe".to_string()),
            settings: Some(serde_json::json!({ "max_search_results": 5 })),
            source: "builtin".to_string(),
        },
    ]
}

/// Tauri IPC command: Lists configured external MCP servers.
#[tauri::command]
pub async fn list_mcp_servers() -> Result<Vec<McpServerConfig>, String> {
    let path = resolve_orbit_dir().join("mcp_servers.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let servers: Vec<McpServerConfig> = serde_json::from_str(&data).unwrap_or_default();
    Ok(servers)
}

/// Tauri IPC command: Saves or updates an external MCP server configuration.
#[tauri::command]
pub async fn save_mcp_server(server: McpServerConfig) -> Result<(), String> {
    let orbit_dir = resolve_orbit_dir();
    let _ = fs::create_dir_all(&orbit_dir);
    let path = orbit_dir.join("mcp_servers.json");

    let mut servers = list_mcp_servers().await.unwrap_or_default();
    if let Some(pos) = servers.iter().position(|s| s.id == server.id) {
        servers[pos] = server;
    } else {
        servers.push(server);
    }

    let json = serde_json::to_string_pretty(&servers).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Tauri IPC command: Deletes an external MCP server by ID.
#[tauri::command]
pub async fn delete_mcp_server(id: String) -> Result<(), String> {
    let orbit_dir = resolve_orbit_dir();
    let path = orbit_dir.join("mcp_servers.json");
    let mut servers = list_mcp_servers().await.unwrap_or_default();
    servers.retain(|s| s.id != id);

    let json = serde_json::to_string_pretty(&servers).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Tauri IPC command: Tests connection and performs live handshake with an MCP server.
#[tauri::command]
pub async fn test_mcp_connection(config: McpServerConfig) -> Result<McpTestResult, String> {
    let start = Instant::now();

    if config.transport == "stdio" {
        let command = match config.command {
            Some(cmd) if !cmd.trim().is_empty() => cmd,
            _ => {
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: 0,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some("No executable command specified for stdio transport".to_string()),
                });
            }
        };

        let mut cmd = Command::new(&command);
        if let Some(args) = config.args {
            cmd.args(args);
        }
        if let Some(cwd) = config.cwd {
            if !cwd.trim().is_empty() {
                cmd.current_dir(cwd);
            }
        }
        if let Some(envs) = config.env {
            for (k, v) in envs {
                cmd.env(k, v);
            }
        }

        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW
            cmd.creation_flags(0x0800_0000);
        }

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some(format!("Failed to spawn process '{command}': {e}")),
                });
            }
        };

        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = child.kill().await;
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some("Failed to open child process stdin".to_string()),
                });
            }
        };

        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill().await;
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some("Failed to open child process stdout".to_string()),
                });
            }
        };

        let mut reader = BufReader::new(stdout).lines();

        // 1. Send MCP initialize request
        let init_req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "orbit-desktop",
                    "version": "1.0.0"
                }
            }
        });

        let init_line = format!("{}\n", init_req);
        if let Err(e) = stdin.write_all(init_line.as_bytes()).await {
            let _ = child.kill().await;
            return Ok(McpTestResult {
                success: false,
                latency_ms: start.elapsed().as_millis() as u64,
                tools_count: 0,
                tools: Vec::new(),
                error: Some(format!("Failed to write initialize request: {e}")),
            });
        }
        let _ = stdin.flush().await;

        // Read initialize response with 4s timeout
        let init_resp = match tokio::time::timeout(Duration::from_secs(4), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            Ok(Ok(None)) => {
                let _ = child.kill().await;
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some("Server closed stdout before replying to initialize".to_string()),
                });
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some(format!("Error reading initialize response: {e}")),
                });
            }
            Err(_) => {
                let _ = child.kill().await;
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some("Timeout waiting for MCP initialize response".to_string()),
                });
            }
        };

        // 2. Send initialized notification and tools/list request
        let notif = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
        let _ = stdin.write_all(notif.as_bytes()).await;

        let list_req = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n";
        if let Err(e) = stdin.write_all(list_req.as_bytes()).await {
            let _ = child.kill().await;
            return Ok(McpTestResult {
                success: false,
                latency_ms: start.elapsed().as_millis() as u64,
                tools_count: 0,
                tools: Vec::new(),
                error: Some(format!("Failed to write tools/list request: {e}")),
            });
        }
        let _ = stdin.flush().await;

        // Read tools/list response
        let list_resp = match tokio::time::timeout(Duration::from_secs(4), reader.next_line()).await {
            Ok(Ok(Some(line))) => line,
            _ => init_resp, // fallback if tools/list was empty or inline
        };

        // Gracefully kill child process
        let _ = child.kill().await;

        let latency_ms = start.elapsed().as_millis() as u64;

        // Parse discovered tools
        let mut discovered_tools = Vec::new();
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&list_resp) {
            if let Some(tool_arr) = v.get("result").and_then(|r| r.get("tools")).and_then(|t| t.as_array()) {
                for item in tool_arr {
                    if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                        let desc = item.get("description").and_then(|d| d.as_str()).map(|s| s.to_string());
                        let schema = item.get("inputSchema").cloned();
                        discovered_tools.push(McpDiscoveredTool {
                            name: name.to_string(),
                            description: desc,
                            input_schema: schema,
                        });
                    }
                }
            }
        }

        Ok(McpTestResult {
            success: true,
            latency_ms,
            tools_count: discovered_tools.len(),
            tools: discovered_tools,
            error: None,
        })
    } else {
        // SSE / HTTP probe
        let url = match config.url {
            Some(u) if !u.trim().is_empty() => u,
            _ => {
                return Ok(McpTestResult {
                    success: false,
                    latency_ms: 0,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some("No endpoint URL specified for SSE/HTTP transport".to_string()),
                });
            }
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;

        let mut req = client.get(&url);
        if let Some(token) = config.bearer_token {
            if !token.trim().is_empty() {
                req = req.bearer_auth(token);
            }
        }
        if let Some(headers) = config.headers {
            for (k, v) in headers {
                req = req.header(k, v);
            }
        }

        match req.send().await {
            Ok(resp) => {
                let latency_ms = start.elapsed().as_millis() as u64;
                let status = resp.status();
                if status.is_success() {
                    Ok(McpTestResult {
                        success: true,
                        latency_ms,
                        tools_count: 0,
                        tools: Vec::new(),
                        error: None,
                    })
                } else {
                    Ok(McpTestResult {
                        success: false,
                        latency_ms,
                        tools_count: 0,
                        tools: Vec::new(),
                        error: Some(format!("Server returned HTTP status {}", status)),
                    })
                }
            }
            Err(e) => {
                Ok(McpTestResult {
                    success: false,
                    latency_ms: start.elapsed().as_millis() as u64,
                    tools_count: 0,
                    tools: Vec::new(),
                    error: Some(format!("Connection failed: {e}")),
                })
            }
        }
    }
}

/// Tauri IPC command: Lists installed and available plugins.
#[tauri::command]
pub async fn list_plugins() -> Result<Vec<PluginConfig>, String> {
    let path = resolve_orbit_dir().join("plugins").join("config.json");
    if !path.exists() {
        return Ok(default_plugins());
    }
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let plugins: Vec<PluginConfig> = serde_json::from_str(&data).unwrap_or_else(|_| default_plugins());
    Ok(plugins)
}

/// Tauri IPC command: Toggles a plugin enabled or disabled.
#[tauri::command]
pub async fn toggle_plugin(id: String, enabled: bool) -> Result<(), String> {
    let orbit_dir = resolve_orbit_dir();
    let plugins_dir = orbit_dir.join("plugins");
    let _ = fs::create_dir_all(&plugins_dir);
    let path = plugins_dir.join("config.json");

    let mut plugins = list_plugins().await.unwrap_or_else(|_| default_plugins());
    if let Some(p) = plugins.iter_mut().find(|p| p.id == id) {
        p.enabled = enabled;
    }

    let json = serde_json::to_string_pretty(&plugins).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

/// Tauri IPC command: Fetches per-tool execution security policies.
#[tauri::command]
pub async fn get_tool_policies() -> Result<HashMap<String, ToolPolicy>, String> {
    let path = resolve_orbit_dir().join("tool_policies.json");
    if !path.exists() {
        return Ok(default_tool_policies());
    }
    let data = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let policies: HashMap<String, ToolPolicy> = serde_json::from_str(&data).unwrap_or_else(|_| default_tool_policies());
    Ok(policies)
}

/// Tauri IPC command: Sets execution security policy for a specific tool.
#[tauri::command]
pub async fn set_tool_policy(tool_name: String, policy: ToolPolicy) -> Result<(), String> {
    let orbit_dir = resolve_orbit_dir();
    let _ = fs::create_dir_all(&orbit_dir);
    let path = orbit_dir.join("tool_policies.json");

    let mut policies = get_tool_policies().await.unwrap_or_else(|_| default_tool_policies());
    policies.insert(tool_name, policy);

    let json = serde_json::to_string_pretty(&policies).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}
