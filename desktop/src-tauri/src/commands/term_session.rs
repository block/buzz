//! Buzz Term handoff: write prompt files and wire per-sid Claude MCP config
//! for (1) OpenClaw workspace grants and (2) buzz-dev-mcp user-signer IPC
//! (reads as signed-in user; writes = Desktop draft only).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::managed_agents::config_bridge::claude::{
    upsert_http_mcp_server, upsert_stdio_mcp_server, OPENCLAW_WORKSPACE_MCP_NAME,
};
use crate::managed_agents::openclaw_workspace_mcp::{
    load_grant, missing_grant_error, OPENCLAW_WORKSPACE_STANDING_INSTRUCTIONS,
};
use crate::managed_agents::resolve_command;
use crate::user_signer::{self, USER_SIGNER_DIR_ENV};

const BUZZ_DEV_MCP_NAME: &str = "buzz-dev-mcp";

/// Standing copy injected when buzz-dev-mcp user-signer is wired for Term.
pub const USER_SIGNER_STANDING_INSTRUCTIONS: &str = "\
## Buzz as signed-in user (Term MCP)

You are acting through buzz-dev-mcp as the **signed-in Desktop user** for Buzz **reads**.
Use MCP tools `buzz_read_thread` / `buzz_read_channel` (Desktop IPC signer — no nsec in this shell).

**Do not message or chat with the human via Buzz.** Do not @mention the user. Do not call
`buzz_draft_message` for your own progress, status, acknowledgements, or status pings —
those appear as the **user's** drafts.

`buzz_draft_message` is **only** for content the human explicitly asked you to prepare for
**them** to send (they click **Send** in Desktop). It is not your outbound channel.

Keep progress and status in this Term TUI / stdout. Never invent an auto-publish path.
Never ask for or echo nsec / BUZZ_PRIVATE_KEY.
";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareTermSessionLaunchInput {
    sid: String,
    tool: String,
    prompt: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    openclaw_workspace: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareTermSessionLaunchResult {
    prompt_path: String,
    claude_config_dir: Option<String>,
    openclaw_wired: bool,
    /// Absolute path to `{app_data}/user-signer` when buzz-dev-mcp was wired.
    user_signer_dir: Option<String>,
    buzz_dev_mcp_wired: bool,
}

fn term_session_root(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir unavailable: {e}"))?;
    Ok(base.join("term-sessions"))
}

fn sanitize_sid(sid: &str) -> Result<String, String> {
    let trimmed = sid.trim();
    if trimmed.is_empty() {
        return Err("sid is required".into());
    }
    if trimmed.len() > 128 {
        return Err("sid is too long".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err("sid has invalid characters".into());
    }
    Ok(trimmed.to_string())
}

fn wire_buzz_dev_mcp(
    app: &AppHandle,
    config_dir: &PathBuf,
) -> Result<(PathBuf, bool), String> {
    let signer_root = user_signer::ensure_root(app)?;
    let mcp_bin = resolve_command("buzz-dev-mcp").ok_or_else(|| {
        "buzz-dev-mcp binary not found (build desktop sidecars / buzz-dev-mcp)".to_string()
    })?;
    let mut env = HashMap::new();
    env.insert(
        USER_SIGNER_DIR_ENV.to_string(),
        signer_root.display().to_string(),
    );
    // Explicitly omit secrets — belt and suspenders; PTY fence already strips them.
    upsert_stdio_mcp_server(
        Some(config_dir),
        BUZZ_DEV_MCP_NAME,
        &mcp_bin.display().to_string(),
        &[],
        Some(&env),
    )?;
    Ok((signer_root, true))
}

/// Write the handoff prompt under app data. For Claude:
/// - always upsert stdio `buzz-dev-mcp` with `BUZZ_USER_SIGNER_DIR` (user IPC)
/// - when `openclawWorkspace`, also upsert OpenClaw HTTP MCP (grant reuse only)
#[tauri::command]
pub fn prepare_term_session_launch(
    app: AppHandle,
    input: PrepareTermSessionLaunchInput,
) -> Result<PrepareTermSessionLaunchResult, String> {
    let sid = sanitize_sid(&input.sid)?;
    let tool = input.tool.trim().to_ascii_lowercase();
    if tool != "claude" && tool != "codex" {
        return Err("tool must be claude or codex".into());
    }
    if input.prompt.is_empty() {
        return Err("prompt is required".into());
    }

    let root = term_session_root(&app)?.join(&sid);
    std::fs::create_dir_all(&root)
        .map_err(|e| format!("failed to create term-session dir: {e}"))?;

    let want_openclaw = input.openclaw_workspace && tool == "claude";
    let mut prompt_body = input.prompt.clone();
    let mut claude_config_dir: Option<String> = None;
    let mut openclaw_wired = false;
    let mut user_signer_dir: Option<String> = None;
    let mut buzz_dev_mcp_wired = false;

    if tool == "claude" {
        let config_dir = root.join("claude-config");
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| format!("failed to create CLAUDE_CONFIG_DIR: {e}"))?;

        match wire_buzz_dev_mcp(&app, &config_dir) {
            Ok((signer_root, wired)) => {
                user_signer_dir = Some(signer_root.display().to_string());
                buzz_dev_mcp_wired = wired;
                if !prompt_body.contains("Buzz as signed-in user (Term MCP)") {
                    let standing = USER_SIGNER_STANDING_INSTRUCTIONS.trim();
                    prompt_body = format!("{standing}\n\n{prompt_body}");
                }
            }
            Err(e) => {
                // Soft-fail: still allow Term open without MCP (toast via UI if needed).
                eprintln!("buzz-desktop: term-session buzz-dev-mcp wire: {e}");
            }
        }

        if want_openclaw {
            let grant = load_grant()?.ok_or_else(missing_grant_error)?;
            upsert_http_mcp_server(
                Some(&config_dir),
                OPENCLAW_WORKSPACE_MCP_NAME,
                &grant.url,
                &grant.authorization,
                grant.headers.as_ref(),
            )?;
            let standing = OPENCLAW_WORKSPACE_STANDING_INSTRUCTIONS.trim();
            if !prompt_body.contains("OpenClaw workspace (Hula)") {
                prompt_body = format!("{standing}\n\n{prompt_body}");
            }
            openclaw_wired = true;
        }

        claude_config_dir = Some(config_dir.display().to_string());
    } else if input.openclaw_workspace && tool == "codex" {
        // Codex has no equivalent MCP config bridge yet — Claude-first gap.
        openclaw_wired = false;
    }

    let _ = input.cwd; // cwd is applied by the shell launch command on the UI side

    let prompt_path = root.join("prompt.txt");
    std::fs::write(&prompt_path, prompt_body.as_bytes())
        .map_err(|e| format!("failed to write prompt file: {e}"))?;

    Ok(PrepareTermSessionLaunchResult {
        prompt_path: prompt_path.display().to_string(),
        claude_config_dir,
        openclaw_wired,
        user_signer_dir,
        buzz_dev_mcp_wired,
    })
}

#[cfg(test)]
mod tests {
    use super::USER_SIGNER_STANDING_INSTRUCTIONS;

    #[test]
    fn user_signer_standing_forbids_progress_drafts() {
        let s = USER_SIGNER_STANDING_INSTRUCTIONS;
        assert!(s.contains("Buzz as signed-in user (Term MCP)"));
        assert!(s.contains("Do not message or chat with the human via Buzz"));
        assert!(s.contains("buzz_draft_message"));
        assert!(s.contains("only"));
        assert!(s.contains("Term TUI") || s.contains("stdout"));
        assert!(!s.contains("To post back into Buzz, call `buzz_draft_message` only"));
    }
}

