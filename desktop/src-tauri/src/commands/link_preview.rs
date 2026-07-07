use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, USER_AGENT},
    redirect::Policy,
};
use url::Url;

const MAX_TITLE_FETCH_BYTES: usize = 256 * 1024;
const TITLE_FETCH_TIMEOUT: Duration = Duration::from_secs(4);
const GITHUB_API_TIMEOUT: Duration = Duration::from_secs(8);

/// Shared HTTP client for GitHub API commands: the PR monitor polls on
/// short intervals, and building a fresh client (connection pool + TLS
/// session cache) per call threw away keep-alive reuse on every tick.
fn github_client() -> Result<&'static reqwest::Client, String> {
    static CLIENT: std::sync::OnceLock<Option<reqwest::Client>> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .pool_idle_timeout(Duration::from_secs(90))
                .pool_max_idle_per_host(2)
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| "github client failed to initialize".to_string())
}

/// Live pull-request details for the rich GitHub PR link card.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubPullRequestInfo {
    pub title: String,
    /// GitHub PR state: `open` or `closed` (merged PRs report `closed`).
    pub state: String,
    pub merged: bool,
    pub draft: bool,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    /// Source branch of the PR (`head.ref`).
    pub head_ref: String,
    /// Head commit sha — used to query check runs.
    pub head_sha: String,
    /// Issue-level comment count.
    pub comments: i64,
    /// Review (inline) comment count.
    pub review_comments: i64,
}

/// Aggregate check-run state for a commit, for the chat work panel's CI
/// monitor.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubCheckSummary {
    pub total: i64,
    pub pending: i64,
    pub failed: i64,
    pub succeeded: i64,
    /// Individual runs for the expanded view (name + coarse state).
    pub runs: Vec<GithubCheckRun>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubCheckRun {
    pub name: String,
    /// `pending` | `success` | `failure`.
    pub state: String,
}

/// Review-thread attention state for a PR: how many threads still await a
/// reply from the PR author. REST cannot see GitHub's "resolved" bit (that
/// is GraphQL-only), so a thread counts as open while its latest comment is
/// from someone other than the PR author — replying clears it, which
/// matches the agent workflow the chat panel automates.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubCommentState {
    pub open_threads: i64,
}

/// Fetch live PR details from the GitHub REST API.
///
/// Anonymous requests cover public repos; when the desktop was launched from
/// a shell with `GITHUB_TOKEN`/`GH_TOKEN` set, the token is attached so
/// private-repo cards work too. Returns `Ok(None)` on any non-success
/// response (not found, rate limited, private without token) — the card
/// falls back to its static form.
#[tauri::command]
pub async fn fetch_github_pull_request(
    owner: String,
    repo: String,
    number: u64,
) -> Result<Option<GithubPullRequestInfo>, String> {
    if !is_valid_github_name(&owner) || !is_valid_github_name(&repo) {
        return Err("invalid GitHub repository reference".to_string());
    }

    let client = github_client()?;

    let url = format!("https://api.github.com/repos/{owner}/{repo}/pulls/{number}");
    let mut request = client
        .get(&url)
        .timeout(GITHUB_API_TIMEOUT)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, "Buzz Desktop link preview")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = ambient_github_token() {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("github request failed: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        // Rate limits and transient failures must NOT read as "no data" —
        // the UI keeps its last good snapshot on error.
        return Err(format!("github request failed: {}", response.status()));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("github response parse failed: {error}"))?;

    Ok(Some(GithubPullRequestInfo {
        title: body["title"].as_str().unwrap_or_default().to_string(),
        state: body["state"].as_str().unwrap_or("open").to_string(),
        merged: body["merged"].as_bool().unwrap_or(false),
        draft: body["draft"].as_bool().unwrap_or(false),
        additions: body["additions"].as_i64().unwrap_or(0),
        deletions: body["deletions"].as_i64().unwrap_or(0),
        changed_files: body["changed_files"].as_i64().unwrap_or(0),
        head_ref: body["head"]["ref"].as_str().unwrap_or_default().to_string(),
        head_sha: body["head"]["sha"].as_str().unwrap_or_default().to_string(),
        comments: body["comments"].as_i64().unwrap_or(0),
        review_comments: body["review_comments"].as_i64().unwrap_or(0),
    }))
}

/// Fetch the check-run summary for a commit. Same auth/fallback behavior as
/// [`fetch_github_pull_request`]: `Ok(None)` on any non-success response.
#[tauri::command]
pub async fn fetch_github_check_summary(
    owner: String,
    repo: String,
    sha: String,
) -> Result<Option<GithubCheckSummary>, String> {
    if !is_valid_github_name(&owner)
        || !is_valid_github_name(&repo)
        || !sha.chars().all(|c| c.is_ascii_hexdigit())
        || sha.is_empty()
        || sha.len() > 64
    {
        return Err("invalid GitHub check reference".to_string());
    }

    let client = github_client()?;

    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/commits/{sha}/check-runs?per_page=100"
    );
    let mut request = client
        .get(&url)
        .timeout(GITHUB_API_TIMEOUT)
        .header(ACCEPT, "application/vnd.github+json")
        .header(USER_AGENT, "Buzz Desktop link preview")
        .header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(token) = ambient_github_token() {
        request = request.header(AUTHORIZATION, format!("Bearer {token}"));
    }

    let response = request
        .send()
        .await
        .map_err(|error| format!("github request failed: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        // Rate limits and transient failures must NOT read as "no data" —
        // the UI keeps its last good snapshot on error.
        return Err(format!("github request failed: {}", response.status()));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| format!("github response parse failed: {error}"))?;

    let raw_runs = body["check_runs"].as_array().cloned().unwrap_or_default();
    let mut pending = 0;
    let mut failed = 0;
    let mut succeeded = 0;
    let mut runs = Vec::with_capacity(raw_runs.len());
    for run in &raw_runs {
        let state = match run["status"].as_str().unwrap_or_default() {
            "completed" => match run["conclusion"].as_str().unwrap_or_default() {
                "success" | "neutral" | "skipped" => {
                    succeeded += 1;
                    "success"
                }
                _ => {
                    failed += 1;
                    "failure"
                }
            },
            _ => {
                pending += 1;
                "pending"
            }
        };
        runs.push(GithubCheckRun {
            name: run["name"].as_str().unwrap_or("check").to_string(),
            state: state.to_string(),
        });
    }

    Ok(Some(GithubCheckSummary {
        total: raw_runs.len() as i64,
        pending,
        failed,
        succeeded,
        runs,
    }))
}

/// Count review threads still awaiting the PR author's reply. See
/// [`GithubCommentState`] for semantics and the REST limitation.
#[tauri::command]
pub async fn fetch_github_pr_comment_state(
    owner: String,
    repo: String,
    number: u64,
) -> Result<Option<GithubCommentState>, String> {
    if !is_valid_github_name(&owner) || !is_valid_github_name(&repo) {
        return Err("invalid GitHub repository reference".to_string());
    }

    let client = github_client()?;

    let base = format!("https://api.github.com/repos/{owner}/{repo}");
    let build = |url: String| {
        let mut request = client
            .get(url)
            .timeout(GITHUB_API_TIMEOUT)
            .header(ACCEPT, "application/vnd.github+json")
            .header(USER_AGENT, "Buzz Desktop link preview")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = ambient_github_token() {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        request
    };

    let pr_response = build(format!("{base}/pulls/{number}"))
        .send()
        .await
        .map_err(|error| format!("github request failed: {error}"))?;
    if pr_response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !pr_response.status().is_success() {
        return Err(format!("github request failed: {}", pr_response.status()));
    }
    let pr: serde_json::Value = pr_response
        .json()
        .await
        .map_err(|error| format!("github response parse failed: {error}"))?;
    let author = pr["user"]["login"].as_str().unwrap_or_default().to_string();

    let comments_response = build(format!(
        "{base}/pulls/{number}/comments?per_page=100&sort=created&direction=asc"
    ))
    .send()
    .await
    .map_err(|error| format!("github request failed: {error}"))?;
    if comments_response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !comments_response.status().is_success() {
        return Err(format!(
            "github request failed: {}",
            comments_response.status()
        ));
    }
    let comments: serde_json::Value = comments_response
        .json()
        .await
        .map_err(|error| format!("github response parse failed: {error}"))?;

    // Group into threads by root comment id; the latest comment (list is
    // created-ascending) decides whether the thread still needs the author.
    let mut last_author_by_thread: std::collections::HashMap<i64, String> =
        std::collections::HashMap::new();
    for comment in comments.as_array().cloned().unwrap_or_default() {
        let id = comment["id"].as_i64().unwrap_or_default();
        let root = comment["in_reply_to_id"].as_i64().unwrap_or(id);
        let login = comment["user"]["login"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        last_author_by_thread.insert(root, login);
    }
    let open_threads = last_author_by_thread
        .values()
        .filter(|login| !author.is_empty() && **login != author)
        .count() as i64;

    Ok(Some(GithubCommentState { open_threads }))
}

/// Discover the pull request for a branch when no PR link has been posted in
/// the chat: read the project's `origin` remote to find the GitHub repo, then
/// look the branch up via the pulls API. Returns the PR's html_url.
#[tauri::command]
pub async fn find_github_pr_for_branch(
    project_path: String,
    branch: String,
) -> Result<Option<String>, String> {
    if branch.is_empty()
        || branch.len() > 255
        || !branch
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err("invalid branch name".to_string());
    }

    let remote = tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(&project_path)
            .args(["remote", "get-url", "origin"])
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|error| format!("git failed to run: {error}"))?;
        if !output.status.success() {
            return Ok::<Option<String>, String>(None);
        }
        Ok(Some(
            String::from_utf8_lossy(&output.stdout).trim().to_string(),
        ))
    })
    .await
    .map_err(|error| format!("git task failed: {error}"))??;

    let Some(remote) = remote else {
        return Ok(None);
    };
    let Some((owner, repo)) = parse_github_remote(&remote) else {
        return Ok(None);
    };
    if !is_valid_github_name(&owner) || !is_valid_github_name(&repo) {
        return Ok(None);
    }

    let client = github_client()?;
    let build = |url: String| {
        let mut request = client
            .get(url)
            .timeout(GITHUB_API_TIMEOUT)
            .header(ACCEPT, "application/vnd.github+json")
            .header(USER_AGENT, "Buzz Desktop link preview")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = ambient_github_token() {
            request = request.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        request
    };

    // Same-repo branches first (the common case), then a scan of open PRs to
    // cover fork-headed pull requests.
    let head = format!("{owner}:{branch}");
    let direct = build(format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls?head={head}&state=all&per_page=1"
    ))
    .send()
    .await
    .map_err(|error| format!("github request failed: {error}"))?;
    if direct.status().is_success() {
        let pulls: serde_json::Value = direct
            .json()
            .await
            .map_err(|error| format!("github response parse failed: {error}"))?;
        if let Some(url) = pulls[0]["html_url"].as_str() {
            return Ok(Some(url.to_string()));
        }
    }

    let open = build(format!(
        "https://api.github.com/repos/{owner}/{repo}/pulls?state=open&per_page=100"
    ))
    .send()
    .await
    .map_err(|error| format!("github request failed: {error}"))?;
    if !open.status().is_success() {
        return Ok(None);
    }
    let pulls: serde_json::Value = open
        .json()
        .await
        .map_err(|error| format!("github response parse failed: {error}"))?;
    for pull in pulls.as_array().cloned().unwrap_or_default() {
        if pull["head"]["ref"].as_str() == Some(branch.as_str()) {
            if let Some(url) = pull["html_url"].as_str() {
                return Ok(Some(url.to_string()));
            }
        }
    }
    Ok(None)
}

/// Extract `(owner, repo)` from a GitHub remote URL — ssh
/// (`git@github.com:owner/repo.git`) or https
/// (`https://github.com/owner/repo[.git]`).
fn parse_github_remote(remote: &str) -> Option<(String, String)> {
    let rest = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .or_else(|| remote.strip_prefix("https://github.com/"))
        .or_else(|| remote.strip_prefix("http://github.com/"))?;
    let mut parts = rest.trim_end_matches('/').splitn(2, '/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.trim_end_matches(".git").to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

fn ambient_github_token() -> Option<String> {
    static TOKEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    TOKEN
        .get_or_init(|| {
            let env_token = ["GITHUB_TOKEN", "GH_TOKEN"].iter().find_map(|name| {
                std::env::var(name)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });
            if env_token.is_some() {
                return env_token;
            }
            // Desktop apps rarely inherit a token env (launched from Finder or
            // a clean shell), but the gh CLI credential is usually present —
            // without it the anonymous 60 req/h limit starves the PR monitor
            // within minutes. Resolved once per process.
            let output = std::process::Command::new("gh")
                .args(["auth", "token"])
                .stdin(std::process::Stdio::null())
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (!token.is_empty()).then_some(token)
        })
        .clone()
}

fn is_valid_github_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[tauri::command]
pub async fn fetch_link_preview_title(href: String) -> Result<Option<String>, String> {
    let url = Url::parse(href.trim()).map_err(|error| format!("invalid URL: {error}"))?;
    if !is_supported_google_link(&url) {
        return Ok(None);
    }

    let client = reqwest::Client::builder()
        .redirect(Policy::none())
        .pool_idle_timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        .map_err(|error| format!("link preview title client failed: {error}"))?;

    let request = client
        .get(url.as_str())
        .header(
            ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header(USER_AGENT, "Buzz Desktop link preview");

    let response = tokio::time::timeout(TITLE_FETCH_TIMEOUT, request.send())
        .await
        .map_err(|_| "link preview title request timed out".to_string())?
        .map_err(|error| format!("link preview title request failed: {error}"))?;

    if !response.status().is_success() {
        return Ok(None);
    }

    let is_html = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("text/html"))
        .unwrap_or(true);
    if !is_html {
        return Ok(None);
    }

    let body = read_limited_text(response).await?;
    Ok(extract_google_title(&body))
}

fn is_supported_google_link(url: &Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }

    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();

    match host.trim_start_matches("www.") {
        "docs.google.com" => {
            matches!(
                segments.as_slice(),
                ["document", "d", _, ..]
                    | ["spreadsheets", "d", _, ..]
                    | ["presentation", "d", _, ..]
            )
        }
        "drive.google.com" => {
            matches!(segments.as_slice(), ["file", "d", _, ..])
                || matches!(segments.as_slice(), ["drive", "folders", _, ..])
                || (segments.first() == Some(&"open")
                    && url.query_pairs().any(|(key, _)| key == "id"))
        }
        _ => false,
    }
}

async fn read_limited_text(response: reqwest::Response) -> Result<String, String> {
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("reading title response failed: {error}"))?;
        if bytes.len() + chunk.len() > MAX_TITLE_FETCH_BYTES {
            let remaining = MAX_TITLE_FETCH_BYTES.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..remaining]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn extract_google_title(html: &str) -> Option<String> {
    extract_meta_title(html)
        .or_else(|| extract_title_tag(html))
        .and_then(|title| normalize_google_title(&title))
}

fn extract_meta_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(relative_start) = lower[search_from..].find("<meta") {
        let start = search_from + relative_start;
        let Some(relative_end) = lower[start..].find('>') else {
            break;
        };
        let end = start + relative_end + 1;
        let tag = &html[start..end];
        let lower_tag = &lower[start..end];

        if lower_tag.contains("og:title") || lower_tag.contains("twitter:title") {
            if let Some(content) = attr_value(tag, "content") {
                return Some(content);
            }
        }

        search_from = end;
    }

    None
}

fn extract_title_tag(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let content_start = start + lower[start..].find('>')? + 1;
    let content_end = content_start + lower[content_start..].find("</title>")?;
    Some(html[content_start..content_end].to_string())
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let attr = attr.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(relative_start) = lower[search_from..].find(&attr) {
        let name_start = search_from + relative_start;
        let name_end = name_start + attr.len();
        let before = lower[..name_start].chars().last();
        let after = lower[name_end..].chars().next();
        let has_name_boundary = !matches!(before, Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_')
            && !matches!(after, Some(c) if c.is_ascii_alphanumeric() || c == '-' || c == '_');

        if has_name_boundary {
            let lower_rest = &lower[name_end..];
            let equals_offset = lower_rest.find('=')?;
            let value_start = name_end + equals_offset + 1;
            let value = tag[value_start..].trim_start();
            let quote = value.chars().next()?;

            if quote == '"' || quote == '\'' {
                let value_body = &value[quote.len_utf8()..];
                let value_end = value_body.find(quote)?;
                return Some(decode_html_entities(&value_body[..value_end]));
            }

            let value_end = value
                .find(|c: char| c.is_ascii_whitespace() || c == '>')
                .unwrap_or(value.len());
            return Some(decode_html_entities(&value[..value_end]));
        }

        search_from = name_end;
    }

    None
}

fn normalize_google_title(raw_title: &str) -> Option<String> {
    let mut title = decode_html_entities(raw_title)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    for suffix in [
        " - Google Docs",
        " - Google Sheets",
        " - Google Slides",
        " - Google Drive",
    ] {
        if let Some(stripped) = title.strip_suffix(suffix) {
            title = stripped.trim().to_string();
            break;
        }
    }

    match title.as_str() {
        ""
        | "Document"
        | "Spreadsheet"
        | "Presentation"
        | "Drive file"
        | "Drive folder"
        | "Google Docs"
        | "Google Sheets"
        | "Google Slides"
        | "Google Drive"
        | "Sign in - Google Accounts" => None,
        _ => Some(title.chars().take(180).collect()),
    }
}

fn decode_html_entities(value: &str) -> String {
    let mut decoded = value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");

    while let Some(start) = decoded.find("&#") {
        let Some(relative_end) = decoded[start..].find(';') else {
            break;
        };
        let end = start + relative_end + 1;
        let entity = &decoded[start + 2..end - 1];
        let parsed = if let Some(hex) = entity
            .strip_prefix('x')
            .or_else(|| entity.strip_prefix('X'))
        {
            u32::from_str_radix(hex, 16).ok()
        } else {
            entity.parse::<u32>().ok()
        };

        let Some(ch) = parsed.and_then(char::from_u32) else {
            break;
        };
        decoded.replace_range(start..end, &ch.to_string());
    }

    decoded
}

#[cfg(test)]
mod tests {
    use super::{extract_google_title, is_supported_google_link};
    use url::Url;

    #[test]
    fn title_prefers_open_graph_title() {
        let html = r#"
          <html>
            <head>
              <meta property="og:title" content="Composer links &amp; previews - Google Docs">
              <title>Fallback - Google Docs</title>
            </head>
          </html>
        "#;

        assert_eq!(
            extract_google_title(html).as_deref(),
            Some("Composer links & previews")
        );
    }

    #[test]
    fn title_ignores_generic_google_titles() {
        assert_eq!(
            extract_google_title("<title>Sign in - Google Accounts</title>"),
            None
        );
        assert_eq!(extract_google_title("<title>Google Docs</title>"), None);
    }

    #[test]
    fn supported_urls_are_google_file_links_only() {
        assert!(is_supported_google_link(
            &Url::parse("https://docs.google.com/document/d/abc/edit").unwrap()
        ));
        assert!(is_supported_google_link(
            &Url::parse("https://docs.google.com/spreadsheets/d/abc/edit").unwrap()
        ));
        assert!(is_supported_google_link(
            &Url::parse("https://drive.google.com/file/d/abc/view").unwrap()
        ));
        assert!(!is_supported_google_link(
            &Url::parse("https://example.com/document/d/abc/edit").unwrap()
        ));
        assert!(!is_supported_google_link(
            &Url::parse("http://docs.google.com/document/d/abc/edit").unwrap()
        ));
    }
}
