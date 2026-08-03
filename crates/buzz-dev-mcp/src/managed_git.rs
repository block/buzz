use rmcp::ErrorData;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt};

const GIT_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_GIT_STREAM_BYTES: usize = 256 * 1024;
const MAX_GIT_ERROR_BYTES: usize = 16 * 1024;
const MAX_RESULT_BYTES: usize = 512 * 1024;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitStatusParams {
    /// File or directory scope, relative to the managed workspace. Defaults to the workspace root.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitDiffParams {
    /// File or directory scope, relative to the managed workspace. Defaults to the workspace root.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
struct GitStatusEntry {
    index: String,
    worktree: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct GitStatusResult {
    schema: u8,
    path: String,
    entries: Vec<GitStatusEntry>,
    truncated: bool,
}

#[derive(Debug, Serialize)]
struct GitDiffResult {
    schema: u8,
    path: String,
    diff: String,
    truncated: bool,
}

#[derive(Debug)]
struct CommandCapture {
    status: Option<ExitStatus>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    truncated: bool,
}

pub(crate) async fn git_status(root: &Path, params: GitStatusParams) -> Result<String, ErrorData> {
    let scope = resolve_scope(root, params.path.as_deref())?;
    let capture = run_git(
        root,
        &[
            "-c",
            "color.ui=false",
            "-c",
            "core.quotepath=true",
            "-c",
            "core.fsmonitor=false",
            "--no-pager",
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignore-submodules=all",
            "--",
            &scope,
        ],
    )
    .await?;
    require_success("status", &capture)?;

    let stdout = String::from_utf8_lossy(&capture.stdout);
    let mut lines = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if capture.truncated && !stdout.ends_with('\n') {
        lines.pop();
    }
    let mut result = GitStatusResult {
        schema: 1,
        path: scope,
        entries: lines.into_iter().map(parse_status_entry).collect(),
        truncated: capture.truncated,
    };
    encode_status_bounded(&mut result)
}

pub(crate) async fn git_diff(root: &Path, params: GitDiffParams) -> Result<String, ErrorData> {
    let scope = resolve_scope(root, params.path.as_deref())?;
    let capture = run_git(
        root,
        &[
            "-c",
            "color.ui=false",
            "-c",
            "core.fsmonitor=false",
            "--no-pager",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=all",
            "--",
            &scope,
        ],
    )
    .await?;
    require_success("diff", &capture)?;

    let mut result = GitDiffResult {
        schema: 1,
        path: scope,
        diff: String::from_utf8_lossy(&capture.stdout).into_owned(),
        truncated: capture.truncated,
    };
    encode_diff_bounded(&mut result)
}

fn resolve_scope(root: &Path, supplied: Option<&str>) -> Result<String, ErrorData> {
    let supplied = supplied.unwrap_or(".");
    if supplied.len() > 4 * 1024 {
        return Err(typed_invalid("invalid_git_path", "path exceeds 4 KiB"));
    }
    let path = Path::new(supplied);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|_| typed_invalid("invalid_git_path", "path is not accessible"))?;
    if resolved != root && !resolved.starts_with(root) {
        return Err(typed_invalid(
            "path_escape",
            "path escapes the managed workspace",
        ));
    }
    let relative = resolved
        .strip_prefix(root)
        .map_err(|_| typed_invalid("path_escape", "path escapes the managed workspace"))?;
    if relative.as_os_str().is_empty() {
        Ok(".".to_owned())
    } else {
        relative
            .to_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| typed_invalid("invalid_git_path", "path is not valid UTF-8"))
    }
}

fn parse_status_entry(line: &str) -> GitStatusEntry {
    let bytes = line.as_bytes();
    let index = bytes.first().copied().unwrap_or(b' ') as char;
    let worktree = bytes.get(1).copied().unwrap_or(b' ') as char;
    let path = line.get(3..).unwrap_or_default().to_owned();
    GitStatusEntry {
        index: index.to_string(),
        worktree: worktree.to_string(),
        path,
    }
}

async fn run_git(root: &Path, args: &[&str]) -> Result<CommandCapture, ErrorData> {
    let mut command = tokio::process::Command::new("git");
    command
        .args(args)
        .current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for key in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_NAMESPACE",
        "GIT_SHALLOW_FILE",
        "GIT_CEILING_DIRECTORIES",
        "GIT_DISCOVERY_ACROSS_FILESYSTEM",
        "GIT_DIFF_OPTS",
        "GIT_EXTERNAL_DIFF",
        "GIT_TRACE",
        "GIT_TRACE_PACK_ACCESS",
        "GIT_TRACE_PACKFILE",
        "GIT_TRACE_PACKET",
        "GIT_TRACE_PERFORMANCE",
        "GIT_TRACE_SETUP",
        "GIT_TRACE_SHALLOW",
        "GIT_TRACE_CURL",
        "GIT_TRACE_CURL_NO_DATA",
        "GIT_TRACE_REDACT",
        "GIT_TRACE_FSMONITOR",
        "GIT_TRACE_REFS",
        "GIT_TRACE2",
        "GIT_TRACE2_EVENT",
        "GIT_TRACE2_PERF",
        "GIT_TRACE2_BRIEF",
        "GIT_TRACE2_CONFIG_PARAMS",
    ] {
        command.env_remove(key);
    }
    crate::configure_no_window_async(&mut command);
    run_bounded_command(command, GIT_TIMEOUT).await
}

async fn run_bounded_command(
    mut command: tokio::process::Command,
    timeout: Duration,
) -> Result<CommandCapture, ErrorData> {
    let mut child = command.spawn().map_err(|_| {
        typed_internal("git_unavailable", "read-only git inspection is unavailable")
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| typed_internal("git_io_error", "read-only git inspection failed"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| typed_internal("git_io_error", "read-only git inspection failed"))?;
    let mut stdout_task = tokio::spawn(read_bounded(stdout, MAX_GIT_STREAM_BYTES));
    let mut stderr_task = tokio::spawn(read_bounded(stderr, MAX_GIT_ERROR_BYTES));
    let mut stdout_result = None;
    let mut stderr_result = None;
    let mut exit_status = None;
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            result = &mut stdout_task, if stdout_result.is_none() => {
                stdout_result = Some(join_reader(result)?);
            }
            result = &mut stderr_task, if stderr_result.is_none() => {
                stderr_result = Some(join_reader(result)?);
            }
            status = child.wait(), if exit_status.is_none() => {
                exit_status = Some(status.map_err(|_| {
                    typed_internal("git_io_error", "read-only git inspection failed")
                })?);
            }
            _ = &mut deadline => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                stdout_task.abort();
                stderr_task.abort();
                return Err(typed_internal("git_timeout", "read-only git inspection timed out"));
            }
        }
        if exit_status.is_some() && stdout_result.is_some() && stderr_result.is_some() {
            break;
        }
    }

    if stdout_result.is_none() {
        stdout_result = Some(join_reader(stdout_task.await)?);
    }
    if stderr_result.is_none() {
        stderr_result = Some(join_reader(stderr_task.await)?);
    }
    let (stdout, stdout_truncated) = stdout_result.expect("stdout reader completed");
    let (stderr, stderr_truncated) = stderr_result.expect("stderr reader completed");
    Ok(CommandCapture {
        status: exit_status,
        stdout,
        stderr,
        truncated: stdout_truncated || stderr_truncated,
    })
}

async fn read_bounded<R: AsyncRead + Unpin>(
    mut reader: R,
    limit: usize,
) -> std::io::Result<(Vec<u8>, bool)> {
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    let mut truncated = false;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        let keep = read.min(remaining);
        bytes.extend_from_slice(&chunk[..keep]);
        truncated |= keep < read;
    }
    Ok((bytes, truncated))
}

fn join_reader(
    result: Result<std::io::Result<(Vec<u8>, bool)>, tokio::task::JoinError>,
) -> Result<(Vec<u8>, bool), ErrorData> {
    result
        .map_err(|_| typed_internal("git_io_error", "read-only git inspection failed"))?
        .map_err(|_| typed_internal("git_io_error", "read-only git inspection failed"))
}

fn require_success(operation: &str, capture: &CommandCapture) -> Result<(), ErrorData> {
    if capture
        .status
        .as_ref()
        .is_some_and(|status| status.success())
    {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&capture.stderr);
    if stderr.to_ascii_lowercase().contains("not a git repository") {
        return Err(typed_invalid(
            "not_git_repository",
            "managed workspace is not a Git repository",
        ));
    }
    Err(typed_internal(
        "git_failed",
        &format!("read-only git {operation} failed"),
    ))
}

fn encode_status_bounded(result: &mut GitStatusResult) -> Result<String, ErrorData> {
    loop {
        let encoded = serde_json::to_string(result)
            .map_err(|_| typed_internal("git_encode_error", "cannot encode git status response"))?;
        if encoded.len() <= MAX_RESULT_BYTES {
            return Ok(encoded);
        }
        if result.entries.pop().is_none() {
            return Err(typed_internal(
                "git_output_too_large",
                "git status response exceeds its output bound",
            ));
        }
        result.truncated = true;
    }
}

fn encode_diff_bounded(result: &mut GitDiffResult) -> Result<String, ErrorData> {
    loop {
        let encoded = serde_json::to_string(result)
            .map_err(|_| typed_internal("git_encode_error", "cannot encode git diff response"))?;
        if encoded.len() <= MAX_RESULT_BYTES {
            return Ok(encoded);
        }
        if result.diff.is_empty() {
            return Err(typed_internal(
                "git_output_too_large",
                "git diff response exceeds its output bound",
            ));
        }
        let mut keep = result.diff.len().saturating_mul(3) / 4;
        while keep > 0 && !result.diff.is_char_boundary(keep) {
            keep -= 1;
        }
        result.diff.truncate(keep);
        result.truncated = true;
    }
}

fn typed_invalid(code: &'static str, message: &str) -> ErrorData {
    ErrorData::invalid_params(message.to_owned(), Some(serde_json::json!({"code": code})))
}

fn typed_internal(code: &'static str, message: &str) -> ErrorData {
    ErrorData::internal_error(message.to_owned(), Some(serde_json::json!({"code": code})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf, process::Command, thread};

    fn setup_repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::managed_files::canonical_root(dir.path().to_owned()).unwrap();
        assert!(Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        fs::write(root.join("tracked.txt"), "base\n").unwrap();
        assert!(Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        (dir, root)
    }

    fn error_code(error: &ErrorData) -> Option<&str> {
        error.data.as_ref()?.get("code")?.as_str()
    }

    #[tokio::test]
    async fn status_and_diff_report_modified_and_untracked_files_without_mutation() {
        let (_dir, root) = setup_repo();
        fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        fs::write(root.join("untracked.txt"), "new\n").unwrap();
        fs::write(root.join("--no-index"), "option-like path\n").unwrap();
        let index_before = fs::read(root.join(".git/index")).unwrap();

        let status = git_status(&root, GitStatusParams { path: None })
            .await
            .unwrap();
        let status: serde_json::Value = serde_json::from_str(&status).unwrap();
        assert!(status["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["path"] == "tracked.txt" && entry["worktree"] == "M" }));
        assert!(status["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["path"] == "untracked.txt" && entry["index"] == "?" }));
        let option_like = git_status(
            &root,
            GitStatusParams {
                path: Some("--no-index".into()),
            },
        )
        .await
        .unwrap();
        let option_like: serde_json::Value = serde_json::from_str(&option_like).unwrap();
        assert_eq!(option_like["entries"][0]["path"], "--no-index");

        let diff = git_diff(&root, GitDiffParams { path: None }).await.unwrap();
        let diff: serde_json::Value = serde_json::from_str(&diff).unwrap();
        assert!(diff["diff"].as_str().unwrap().contains("-base"));
        assert!(diff["diff"].as_str().unwrap().contains("+changed"));
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);
        assert_eq!(
            fs::read_to_string(root.join("untracked.txt")).unwrap(),
            "new\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("--no-index")).unwrap(),
            "option-like path\n"
        );
    }

    #[tokio::test]
    async fn non_repo_returns_stable_error() {
        let dir = tempfile::tempdir().unwrap();
        let root = crate::managed_files::canonical_root(dir.path().to_owned()).unwrap();
        let error = git_status(&root, GitStatusParams { path: None })
            .await
            .unwrap_err();
        assert_eq!(error_code(&error), Some("not_git_repository"));
        let error = git_diff(&root, GitDiffParams { path: None })
            .await
            .unwrap_err();
        assert_eq!(error_code(&error), Some("not_git_repository"));
    }

    #[tokio::test]
    async fn rejects_absolute_escape() {
        let (_dir, root) = setup_repo();
        let outside = tempfile::NamedTempFile::new().unwrap();
        let error = git_status(
            &root,
            GitStatusParams {
                path: Some(outside.path().display().to_string()),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error_code(&error), Some("path_escape"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;
        let (_dir, root) = setup_repo();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.join("escape")).unwrap();
        let error = git_diff(
            &root,
            GitDiffParams {
                path: Some("escape".into()),
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error_code(&error), Some("path_escape"));
    }

    #[tokio::test]
    async fn diff_response_is_bounded() {
        let (_dir, root) = setup_repo();
        fs::write(
            root.join("tracked.txt"),
            "x".repeat(MAX_GIT_STREAM_BYTES * 2),
        )
        .unwrap();
        let diff = git_diff(&root, GitDiffParams { path: None }).await.unwrap();
        assert!(diff.len() <= MAX_RESULT_BYTES);
        let diff: serde_json::Value = serde_json::from_str(&diff).unwrap();
        assert_eq!(diff["truncated"], true);
    }

    #[test]
    #[ignore]
    fn timeout_child() {
        thread::sleep(Duration::from_secs(30));
    }

    #[tokio::test]
    async fn command_timeout_returns_stable_error() {
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "managed_git::tests::timeout_child",
                "--ignored",
                "--nocapture",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let error = run_bounded_command(command, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert_eq!(error_code(&error), Some("git_timeout"));
    }
}
