use std::process::Stdio;
use tokio::io::AsyncReadExt;

async fn github(args: &[&str]) -> Result<serde_json::Value, String> {
    let executable = crate::managed_agents::resolve_command("gh")
        .ok_or("Install GitHub CLI and run gh auth login to view GitHub tasks")?;
    let mut child = tokio::process::Command::new(executable)
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_HOST", "github.com")
        .env_remove("BUZZ_PRIVATE_KEY")
        .env_remove("BUZZ_AUTH_TAG")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("Missing GitHub stdout")?;
    let stderr = child.stderr.take().ok_or("Missing GitHub stderr")?;
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        let mut output = Vec::new();
        let mut error = Vec::new();
        let mut stdout = stdout.take(8 * 1024 * 1024);
        let mut stderr = stderr.take(64 * 1024);
        tokio::try_join!(
            stdout.read_to_end(&mut output),
            stderr.read_to_end(&mut error),
        )
        .map_err(|e| e.to_string())?;
        if output.len() == 8 * 1024 * 1024 || error.len() == 64 * 1024 {
            return Err("GitHub response exceeded the preview limit".into());
        }
        if !child.wait().await.map_err(|e| e.to_string())?.success() {
            return Err(String::from_utf8_lossy(&error).trim().to_string());
        }
        serde_json::from_slice(&output).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "GitHub request timed out".to_string())?
}

/// Read a branch's GitHub changes or pull requests using the local gh login.
#[tauri::command]
pub async fn get_task_github(
    repository: String,
    branch: String,
    view: String,
) -> Result<serde_json::Value, String> {
    let parts: Vec<_> = repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || p.starts_with('.')
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        })
    {
        return Err("Expected GitHub owner/repository".into());
    }
    if branch.is_empty() || branch.len() > 1024 {
        return Err("Expected a branch name".into());
    }
    let prs = github(&[
        "pr",
        "list",
        "--repo",
        &repository,
        "--head",
        &branch,
        "--state",
        "all",
        "--limit",
        "20",
        "--json",
        "number,title,body,url,state,isDraft,headRefOid,baseRefName,reviewDecision,statusCheckRollup",
    ])
    .await?;
    if view == "review" {
        return Ok(prs);
    }
    if view != "changes" {
        return Err("Unknown task view".into());
    }
    let prs = prs.as_array().ok_or("Invalid GitHub PR response")?;
    let pr = prs
        .iter()
        .find(|pr| pr["state"] == "OPEN")
        .or_else(|| prs.first());
    let base = match pr.and_then(|pr| pr["baseRefName"].as_str()) {
        Some(base) => base.to_string(),
        None => github(&["repo", "view", &repository, "--json", "defaultBranchRef"]).await?
            ["defaultBranchRef"]["name"]
            .as_str()
            .ok_or("Repository has no default branch")?
            .to_string(),
    };
    let encode =
        |value: &str| url::form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
    github(&[
        "api",
        "--hostname",
        "github.com",
        &format!(
            "repos/{repository}/compare/{}...{}",
            encode(&base),
            encode(&branch)
        ),
    ])
    .await
}
