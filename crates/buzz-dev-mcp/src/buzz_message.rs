use crate::shell::SharedState;
use rmcp::model::{CallToolResult, Content};
use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

const MAX_CONTENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageParams {
    /// Buzz channel UUID supplied in the turn's Context section.
    pub channel: String,
    /// Message body to publish.
    pub content: String,
    /// Optional event id to reply to when the Context section requires a threaded reply.
    #[serde(default)]
    pub reply_to: Option<String>,
}

pub async fn run(
    state: &SharedState,
    params: SendMessageParams,
) -> Result<CallToolResult, ErrorData> {
    if params.content.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "content must not be empty".to_string(),
            None,
        ));
    }
    if params.content.len() > MAX_CONTENT_BYTES {
        return Err(ErrorData::invalid_params(
            format!("content exceeds {MAX_CONTENT_BYTES} bytes"),
            None,
        ));
    }

    let buzz = find_buzz(&state.shim.path_env).ok_or_else(|| {
        ErrorData::internal_error("bundled Buzz CLI is unavailable".to_string(), None)
    })?;
    let mut command = Command::new(buzz);
    command
        .args([
            "messages",
            "send",
            "--channel",
            &params.channel,
            "--content",
            &params.content,
        ])
        .current_dir(&state.cwd)
        .env("PATH", &state.shim.path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(reply_to) = params.reply_to.as_deref() {
        command.args(["--reply-to", reply_to]);
    }

    let output = command.output().await.map_err(|error| {
        ErrorData::internal_error(format!("failed to run Buzz CLI: {error}"), None)
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        let text = if stdout.is_empty() {
            "Message published.".to_string()
        } else {
            stdout
        };
        Ok(CallToolResult::success(vec![Content::text(text)]))
    } else {
        let detail = if stderr.is_empty() { stdout } else { stderr };
        Ok(CallToolResult::error(vec![Content::text(format!(
            "Buzz message failed: {detail}"
        ))]))
    }
}

fn find_buzz(path: &str) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|entry| entry.join(if cfg!(windows) { "buzz.exe" } else { "buzz" }))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn make_executable(path: &std::path::Path) {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn publishes_with_structured_cli_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let buzz = dir
            .path()
            .join(if cfg!(windows) { "buzz.cmd" } else { "buzz" });
        let args_file = dir.path().join("args.txt");
        if cfg!(windows) {
            std::fs::write(
                &buzz,
                format!(
                    "@echo off\r\n(for %%a in (%*) do @echo %%~a)>>\"{}\"\r\n",
                    args_file.display()
                ),
            )
            .unwrap();
        } else {
            std::fs::write(
                &buzz,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n",
                    args_file.display()
                ),
            )
            .unwrap();
        }
        make_executable(&buzz);
        let mut state = SharedState::new(
            dir.path().to_path_buf(),
            crate::shim::Shim::install().unwrap(),
        )
        .unwrap();
        state.shim.path_env = dir.path().to_string_lossy().into_owned();
        let result = run(
            &state,
            SendMessageParams {
                channel: "channel-id".into(),
                content: "hello world".into(),
                reply_to: Some("event-id".into()),
            },
        )
        .await
        .unwrap();
        assert!(!result.is_error.unwrap_or(false));
        let args = std::fs::read_to_string(args_file).unwrap();
        assert_eq!(
            args.lines().collect::<Vec<_>>(),
            [
                "messages",
                "send",
                "--channel",
                "channel-id",
                "--content",
                "hello world",
                "--reply-to",
                "event-id"
            ]
        );
    }

    #[test]
    fn finds_bundled_buzz_on_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(if cfg!(windows) { "buzz.exe" } else { "buzz" });
        std::fs::write(&path, "test").unwrap();
        assert_eq!(find_buzz(&dir.path().to_string_lossy()), Some(path));
    }
}
