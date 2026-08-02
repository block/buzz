use std::{sync::OnceLock, time::Duration};

use bytes::BytesMut;
use futures_util::StreamExt;
use reqwest::{header::HeaderMap, Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use url::Url;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    app_state::{keyring_service, AppState},
    secret_store::SecretStore,
};

const CLICKUP_API_BASE: &str = "https://api.clickup.com/api/v2/";
const CLICKUP_RESPONSE_LIMIT_BYTES: usize = 10 * 1024 * 1024;
const MAX_TASK_PAGES: u32 = 20;
const MAX_TASKS: usize = 2_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpUser {
    id: u64,
    username: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    profile_picture: Option<String>,
    #[serde(default)]
    initials: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClickUpUserResponse {
    user: ClickUpUser,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClickUpAuthStatus {
    connected: bool,
    account: Option<ClickUpUser>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpWorkspace {
    id: String,
    name: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClickUpWorkspacesResponse {
    #[serde(default)]
    teams: Vec<ClickUpWorkspace>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct ClickUpTaskStatus {
    #[serde(default)]
    status: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpTaskPriority {
    #[serde(default)]
    priority: String,
    #[serde(default)]
    color: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpNamedLocation {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpTaskTag {
    #[serde(default)]
    name: String,
    #[serde(default)]
    tag_fg: Option<String>,
    #[serde(default)]
    tag_bg: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpCustomField {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    field_type: String,
    #[serde(default)]
    value: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpDependency {
    #[serde(default)]
    task_id: Option<String>,
    #[serde(default)]
    depends_on: Option<String>,
    #[serde(default)]
    dependency_of: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpTask {
    id: String,
    name: String,
    #[serde(default)]
    text_content: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    status: ClickUpTaskStatus,
    #[serde(default)]
    priority: Option<ClickUpTaskPriority>,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    start_date: Option<String>,
    #[serde(default)]
    date_created: Option<String>,
    #[serde(default)]
    date_updated: Option<String>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    team_id: String,
    #[serde(default)]
    list: Option<ClickUpNamedLocation>,
    #[serde(default)]
    folder: Option<ClickUpNamedLocation>,
    #[serde(default)]
    space: Option<ClickUpNamedLocation>,
    #[serde(default)]
    assignees: Vec<ClickUpUser>,
    #[serde(default)]
    tags: Vec<ClickUpTaskTag>,
    #[serde(default)]
    subtasks: Vec<ClickUpTask>,
    #[serde(default)]
    custom_fields: Vec<ClickUpCustomField>,
    #[serde(default)]
    dependencies: Vec<ClickUpDependency>,
}

#[derive(Debug, Deserialize)]
struct ClickUpTasksResponse {
    #[serde(default)]
    tasks: Vec<ClickUpTask>,
    #[serde(default)]
    last_page: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct ClickUpTaskPage {
    tasks: Vec<ClickUpTask>,
    fetched_at_ms: u64,
    truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpCommentPart {
    #[serde(default)]
    text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpComment {
    id: Value,
    #[serde(default)]
    comment_text: Option<String>,
    #[serde(default)]
    comment: Vec<ClickUpCommentPart>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    user: Option<ClickUpUser>,
    #[serde(default)]
    resolved: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct ClickUpCommentsResponse {
    #[serde(default)]
    comments: Vec<ClickUpComment>,
}

struct ClickUpApi {
    base_url: Url,
    client: reqwest::Client,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn clickup_error(code: &str, message: &str, retry_at_ms: Option<u64>) -> String {
    format!(
        "clickup:{code}:{}:{message}",
        retry_at_ms
            .map(|value| value.to_string())
            .unwrap_or_default()
    )
}

fn retry_at_ms(headers: &HeaderMap) -> Option<u64> {
    if let Some(value) = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    {
        return Some(if value < 1_000_000_000_000 {
            value.saturating_mul(1_000)
        } else {
            value
        });
    }
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| now_ms().saturating_add(seconds.saturating_mul(1_000)))
}

impl ClickUpApi {
    fn new(base_url: &str) -> Result<Self, String> {
        let base_url = Url::parse(base_url)
            .map_err(|_| clickup_error("invalid_request", "ClickUp API URL is invalid.", None))?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| {
                clickup_error(
                    "network",
                    "Buzz could not initialize the ClickUp client.",
                    None,
                )
            })?;
        Ok(Self { base_url, client })
    }

    fn endpoint(&self, segments: &[&str]) -> Result<Url, String> {
        let mut endpoint = self.base_url.clone();
        {
            let mut path = endpoint.path_segments_mut().map_err(|_| {
                clickup_error("invalid_request", "ClickUp API URL is invalid.", None)
            })?;
            path.pop_if_empty();
            path.extend(segments.iter().copied());
        }
        Ok(endpoint)
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        token: &str,
        method: Method,
        segments: &[&str],
        query: &[(String, String)],
    ) -> Result<T, String> {
        let endpoint = self.endpoint(segments)?;
        let response = self
            .client
            .request(method, endpoint)
            .header(reqwest::header::AUTHORIZATION, token)
            .query(query)
            .send()
            .await
            .map_err(|_| clickup_error("network", "Buzz could not reach ClickUp.", None))?;
        let status = response.status();
        let headers = response.headers().clone();

        if status.is_redirection() {
            return Err(clickup_error(
                "redirect_rejected",
                "ClickUp returned an unexpected redirect.",
                None,
            ));
        }
        if status == StatusCode::UNAUTHORIZED {
            return Err(clickup_error(
                "unauthorized",
                "ClickUp rejected the personal token.",
                None,
            ));
        }
        if status == StatusCode::FORBIDDEN {
            return Err(clickup_error(
                "forbidden",
                "The connected ClickUp account cannot access this resource.",
                None,
            ));
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(clickup_error(
                "rate_limited",
                "ClickUp is temporarily limiting requests.",
                retry_at_ms(&headers),
            ));
        }
        if status.is_server_error() {
            return Err(clickup_error(
                "server",
                "ClickUp is temporarily unavailable.",
                None,
            ));
        }
        if !status.is_success() {
            return Err(clickup_error(
                "invalid_request",
                "ClickUp could not complete this read request.",
                None,
            ));
        }

        if headers
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok())
            .is_some_and(|size| size > CLICKUP_RESPONSE_LIMIT_BYTES)
        {
            return Err(clickup_error(
                "response_too_large",
                "ClickUp returned more data than Buzz can safely display.",
                None,
            ));
        }

        let mut bytes = BytesMut::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                clickup_error("network", "Buzz could not read the ClickUp response.", None)
            })?;
            if bytes.len().saturating_add(chunk.len()) > CLICKUP_RESPONSE_LIMIT_BYTES {
                return Err(clickup_error(
                    "response_too_large",
                    "ClickUp returned more data than Buzz can safely display.",
                    None,
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| clickup_error("server", "ClickUp returned an unreadable response.", None))
    }

    async fn current_user(&self, token: &str) -> Result<ClickUpUser, String> {
        let response: ClickUpUserResponse = self
            .request_json(token, Method::GET, &["user"], &[])
            .await?;
        Ok(response.user)
    }

    async fn workspaces(&self, token: &str) -> Result<Vec<ClickUpWorkspace>, String> {
        let response: ClickUpWorkspacesResponse = self
            .request_json(token, Method::GET, &["team"], &[])
            .await?;
        Ok(response.teams)
    }

    async fn tasks(
        &self,
        token: &str,
        workspace_id: &str,
        assignee_id: u64,
    ) -> Result<ClickUpTaskPage, String> {
        validate_resource_id(workspace_id, "Workspace")?;
        let mut tasks = Vec::new();
        let mut truncated = false;
        for page in 0..MAX_TASK_PAGES {
            let query = vec![
                ("assignees[]".to_string(), assignee_id.to_string()),
                ("include_closed".to_string(), "false".to_string()),
                ("subtasks".to_string(), "true".to_string()),
                ("order_by".to_string(), "due_date".to_string()),
                ("page".to_string(), page.to_string()),
            ];
            let response: ClickUpTasksResponse = self
                .request_json(token, Method::GET, &["team", workspace_id, "task"], &query)
                .await?;
            let is_empty = response.tasks.is_empty();
            let remaining = MAX_TASKS.saturating_sub(tasks.len());
            if response.tasks.len() > remaining {
                tasks.extend(response.tasks.into_iter().take(remaining));
                return Ok(ClickUpTaskPage {
                    tasks,
                    fetched_at_ms: now_ms(),
                    truncated: true,
                });
            }
            tasks.extend(response.tasks);
            if response.last_page || is_empty {
                return Ok(ClickUpTaskPage {
                    tasks,
                    fetched_at_ms: now_ms(),
                    truncated,
                });
            }
            if page + 1 == MAX_TASK_PAGES {
                truncated = true;
            }
        }
        Ok(ClickUpTaskPage {
            tasks,
            fetched_at_ms: now_ms(),
            truncated,
        })
    }

    async fn task(&self, token: &str, task_id: &str) -> Result<ClickUpTask, String> {
        validate_resource_id(task_id, "Task")?;
        let query = vec![("include_subtasks".to_string(), "true".to_string())];
        self.request_json(token, Method::GET, &["task", task_id], &query)
            .await
    }

    async fn comments(
        &self,
        token: &str,
        task_id: &str,
    ) -> Result<ClickUpCommentsResponse, String> {
        validate_resource_id(task_id, "Task")?;
        self.request_json(token, Method::GET, &["task", task_id, "comment"], &[])
            .await
    }
}

fn production_api() -> Result<&'static ClickUpApi, String> {
    static API: OnceLock<Result<ClickUpApi, String>> = OnceLock::new();
    match API.get_or_init(|| ClickUpApi::new(CLICKUP_API_BASE)) {
        Ok(api) => Ok(api),
        Err(error) => Err(error.clone()),
    }
}

fn validate_resource_id(value: &str, label: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(clickup_error(
            "invalid_request",
            &format!("{label} identifier is invalid."),
            None,
        ));
    }
    Ok(())
}

fn normalize_personal_token(value: &str) -> Result<String, String> {
    let token = value.trim();
    if token.len() < 12
        || token.len() > 512
        || !token.starts_with("pk_")
        || token.chars().any(char::is_whitespace)
    {
        return Err(clickup_error(
            "invalid_token",
            "Enter a valid ClickUp personal token beginning with pk_.",
            None,
        ));
    }
    Ok(token.to_owned())
}

fn credential_key(state: &AppState) -> Result<String, String> {
    let keys = state.signing_keys().map_err(|_| {
        clickup_error(
            "keyring_unavailable",
            "Buzz cannot securely scope the ClickUp credential right now.",
            None,
        )
    })?;
    Ok(format!(
        "clickup:{}:personal-token",
        keys.public_key().to_hex()
    ))
}

fn credential_store() -> &'static SecretStore {
    SecretStore::shared(keyring_service())
}

trait CredentialBackend {
    fn load_value(&self, key: &str) -> Result<Option<String>, ()>;
    fn store_value(&self, key: &str, value: &str) -> Result<(), ()>;
    fn verify_value(&self, key: &str, expected: &str) -> Result<bool, ()>;
    fn delete_value(&self, key: &str) -> Result<(), ()>;
}

impl CredentialBackend for SecretStore {
    fn load_value(&self, key: &str) -> Result<Option<String>, ()> {
        self.load(key).map_err(|_| ())
    }

    fn store_value(&self, key: &str, value: &str) -> Result<(), ()> {
        self.store(key, value).map_err(|_| ())
    }

    fn verify_value(&self, key: &str, expected: &str) -> Result<bool, ()> {
        self.verify_stored_raw(key, expected).map_err(|_| ())
    }

    fn delete_value(&self, key: &str) -> Result<(), ()> {
        self.delete(key).map_err(|_| ())
    }
}

fn restore_previous_token(
    store: &impl CredentialBackend,
    key: &str,
    previous: Option<&str>,
) -> Result<(), ()> {
    match previous {
        Some(value) => {
            store.store_value(key, value)?;
            if store.verify_value(key, value)? {
                Ok(())
            } else {
                Err(())
            }
        }
        None => {
            store.delete_value(key)?;
            if store.load_value(key)?.is_none() {
                Ok(())
            } else {
                Err(())
            }
        }
    }
}

fn persist_token_transactionally(
    store: &impl CredentialBackend,
    key: &str,
    token: &str,
) -> Result<(), String> {
    let previous = store
        .load_value(key)
        .map_err(|_| {
            clickup_error(
                "keyring_unavailable",
                "Buzz could not access secure keyring storage.",
                None,
            )
        })?
        .map(Zeroizing::new);

    let stored = store.store_value(key, token);
    let verified = stored
        .as_ref()
        .map(|()| store.verify_value(key, token))
        .unwrap_or(Err(()));
    if stored.is_ok() && matches!(verified, Ok(true)) {
        return Ok(());
    }

    if restore_previous_token(store, key, previous.as_ref().map(|value| value.as_str())).is_err() {
        return Err(clickup_error(
            "keyring_unavailable",
            "Buzz could not verify or restore the ClickUp token in secure storage.",
            None,
        ));
    }
    Err(clickup_error(
        "keyring_unavailable",
        "Buzz could not verify the ClickUp token in secure storage. The previous credential was restored.",
        None,
    ))
}

fn enforce_task_scope(
    task: &ClickUpTask,
    workspace_id: &str,
    assignee_id: u64,
) -> Result<(), String> {
    validate_resource_id(workspace_id, "Workspace")?;
    let assigned = task.assignees.iter().any(|user| user.id == assignee_id);
    let closed = task
        .status
        .kind
        .as_deref()
        .is_some_and(|kind| matches!(kind, "closed" | "done"));
    if task.team_id != workspace_id || !assigned || task.archived || closed {
        return Err(clickup_error(
            "forbidden",
            "This task is outside the selected read-only My Work scope.",
            None,
        ));
    }
    Ok(())
}

fn ensure_credential_key_unchanged(expected: &str, current: &str) -> Result<(), String> {
    if expected == current {
        return Ok(());
    }
    Err(clickup_error(
        "identity_changed",
        "The active Buzz identity changed while ClickUp was connecting. Enter the token again for the current identity.",
        None,
    ))
}

fn load_token(state: &AppState) -> Result<Option<Zeroizing<String>>, String> {
    let key = credential_key(state)?;
    credential_store()
        .load(&key)
        .map(|token| token.map(Zeroizing::new))
        .map_err(|_| {
            clickup_error(
                "keyring_unavailable",
                "Buzz could not access secure keyring storage.",
                None,
            )
        })
}

fn require_token(state: &AppState) -> Result<Zeroizing<String>, String> {
    load_token(state)?.ok_or_else(|| {
        clickup_error(
            "not_connected",
            "Connect ClickUp before loading tasks.",
            None,
        )
    })
}

#[tauri::command]
pub(crate) async fn clickup_auth_status(
    state: State<'_, AppState>,
) -> Result<ClickUpAuthStatus, String> {
    let Some(token) = load_token(&state)? else {
        return Ok(ClickUpAuthStatus {
            connected: false,
            account: None,
        });
    };
    let account = production_api()?.current_user(&token).await?;
    Ok(ClickUpAuthStatus {
        connected: true,
        account: Some(account),
    })
}

#[tauri::command]
pub(crate) async fn clickup_connect(
    personal_token: String,
    state: State<'_, AppState>,
) -> Result<ClickUpAuthStatus, String> {
    let key = credential_key(&state)?;
    let mut incoming = Zeroizing::new(personal_token);
    let normalized = normalize_personal_token(&incoming)?;
    incoming.zeroize();
    let token = Zeroizing::new(normalized);
    let account = production_api()?.current_user(&token).await?;
    ensure_credential_key_unchanged(&key, &credential_key(&state)?)?;
    let store = credential_store();
    persist_token_transactionally(store, &key, &token)?;
    Ok(ClickUpAuthStatus {
        connected: true,
        account: Some(account),
    })
}

#[tauri::command]
pub(crate) fn clickup_disconnect(state: State<'_, AppState>) -> Result<(), String> {
    let key = credential_key(&state)?;
    let store = credential_store();
    store.delete(&key).map_err(|_| {
        clickup_error(
            "keyring_unavailable",
            "Buzz could not remove the ClickUp token from secure storage.",
            None,
        )
    })?;
    if store
        .load(&key)
        .map_err(|_| {
            clickup_error(
                "keyring_unavailable",
                "Buzz could not verify ClickUp disconnection.",
                None,
            )
        })?
        .is_some()
    {
        return Err(clickup_error(
            "keyring_unavailable",
            "Buzz could not verify ClickUp disconnection.",
            None,
        ));
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn clickup_list_workspaces(
    state: State<'_, AppState>,
) -> Result<Vec<ClickUpWorkspace>, String> {
    let token = require_token(&state)?;
    production_api()?.workspaces(&token).await
}

#[tauri::command]
pub(crate) async fn clickup_list_tasks(
    workspace_id: String,
    state: State<'_, AppState>,
) -> Result<ClickUpTaskPage, String> {
    let token = require_token(&state)?;
    let api = production_api()?;
    let account = api.current_user(&token).await?;
    api.tasks(&token, &workspace_id, account.id).await
}

#[tauri::command]
pub(crate) async fn clickup_get_task(
    workspace_id: String,
    task_id: String,
    state: State<'_, AppState>,
) -> Result<ClickUpTask, String> {
    let token = require_token(&state)?;
    let api = production_api()?;
    let account = api.current_user(&token).await?;
    let task = api.task(&token, &task_id).await?;
    enforce_task_scope(&task, &workspace_id, account.id)?;
    Ok(task)
}

#[tauri::command]
pub(crate) async fn clickup_get_task_comments(
    workspace_id: String,
    task_id: String,
    state: State<'_, AppState>,
) -> Result<ClickUpCommentsResponse, String> {
    let token = require_token(&state)?;
    let api = production_api()?;
    let account = api.current_user(&token).await?;
    let task = api.task(&token, &task_id).await?;
    enforce_task_scope(&task, &workspace_id, account.id)?;
    api.comments(&token, &task_id).await
}

#[cfg(test)]
#[path = "clickup_security_tests.rs"]
mod security_tests;

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, convert::Infallible};

    use axum::{
        body::Body,
        extract::Query,
        http::HeaderMap as AxumHeaderMap,
        response::{Redirect, Response},
        routing::get,
        Json, Router,
    };
    use futures_util::stream;
    use serde_json::json;

    use super::*;

    #[test]
    fn personal_tokens_are_trimmed_and_validated() {
        assert_eq!(
            normalize_personal_token("  pk_123456789012  ").unwrap(),
            "pk_123456789012"
        );
        assert!(normalize_personal_token("not-a-token").is_err());
        assert!(normalize_personal_token("pk_with whitespace").is_err());
    }

    #[test]
    fn resource_ids_reject_path_and_query_injection() {
        assert!(validate_resource_id("86abc-123", "Task").is_ok());
        assert!(validate_resource_id("../user", "Task").is_err());
        assert!(validate_resource_id("team?admin=true", "Workspace").is_err());
    }

    async fn spawn_test_api(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{address}/api/v2/"), handle)
    }

    #[tokio::test]
    async fn authenticated_task_reads_paginate_until_last_page() {
        async fn user(headers: AxumHeaderMap) -> (StatusCode, Json<Value>) {
            if headers
                .get(reqwest::header::AUTHORIZATION)
                .and_then(|value| value.to_str().ok())
                != Some("pk_test-token")
            {
                return (StatusCode::UNAUTHORIZED, Json(json!({})));
            }
            (
                StatusCode::OK,
                Json(json!({ "user": { "id": 42, "username": "Mikes" } })),
            )
        }

        async fn tasks(Query(query): Query<HashMap<String, String>>) -> Json<Value> {
            let page = query.get("page").map(String::as_str).unwrap_or("0");
            let id = if page == "0" { "first" } else { "second" };
            Json(json!({
                "tasks": [{
                    "id": id,
                    "name": format!("Task {id}"),
                    "status": { "status": "open" },
                    "team_id": "123"
                }],
                "last_page": page == "1"
            }))
        }

        let router = Router::new()
            .route("/api/v2/user", get(user))
            .route("/api/v2/team/{workspace_id}/task", get(tasks));
        let (base_url, handle) = spawn_test_api(router).await;
        let api = ClickUpApi::new(&base_url).unwrap();
        let user = api.current_user("pk_test-token").await.unwrap();
        let result = api.tasks("pk_test-token", "123", user.id).await.unwrap();
        handle.abort();

        assert_eq!(
            result
                .tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn redirects_are_rejected_without_following() {
        let router = Router::new().route(
            "/api/v2/user",
            get(|| async { Redirect::temporary("https://example.com/steal") }),
        );
        let (base_url, handle) = spawn_test_api(router).await;
        let api = ClickUpApi::new(&base_url).unwrap();
        let error = api.current_user("pk_test-token").await.unwrap_err();
        handle.abort();

        assert!(error.starts_with("clickup:redirect_rejected:"));
    }

    #[tokio::test]
    async fn chunked_responses_are_stopped_at_the_byte_limit() {
        let router = Router::new().route(
            "/api/v2/user",
            get(|| async {
                let chunks = stream::iter([
                    Ok::<_, Infallible>(bytes::Bytes::from(vec![b'a'; 6 * 1024 * 1024])),
                    Ok::<_, Infallible>(bytes::Bytes::from(vec![b'b'; 6 * 1024 * 1024])),
                ]);
                Response::new(Body::from_stream(chunks))
            }),
        );
        let (base_url, handle) = spawn_test_api(router).await;
        let api = ClickUpApi::new(&base_url).unwrap();
        let error = api.current_user("pk_test-token").await.unwrap_err();
        handle.abort();

        assert!(error.starts_with("clickup:response_too_large:"));
    }

    #[tokio::test]
    async fn rate_limit_reset_is_preserved_for_the_ui() {
        let router = Router::new().route(
            "/api/v2/user",
            get(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("x-ratelimit-reset", "2000000000")],
                    Json(json!({})),
                )
            }),
        );
        let (base_url, handle) = spawn_test_api(router).await;
        let api = ClickUpApi::new(&base_url).unwrap();
        let error = api.current_user("pk_test-token").await.unwrap_err();
        handle.abort();

        assert!(error.starts_with("clickup:rate_limited:2000000000000:"));
    }
}
