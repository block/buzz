#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, Mutex, Notify},
};
use url::Url;

const PROTOCOL_VERSION: u64 = 2;
const HOST_FINAL_REPLY_VERSION: u64 = 1;
const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;
const MAX_PROMPT_BYTES: usize = 512 * 1024;
const MAX_ERROR_DETAIL_CHARS: usize = 512;
const PROGRESS_INTERVAL: Duration = Duration::from_secs(15);
const STOP_ACKNOWLEDGEMENT: &str = "Manual agent run stopped by request.";
const NON_RETRYABLE_REMOTE_TURN_CODE: i64 = -32041;

#[derive(Clone, Debug)]
struct Config {
    base_url: Url,
    token: String,
    poll_interval: Duration,
    run_timeout: Duration,
    model: Option<String>,
    reasoning_effort: Option<String>,
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let base_url = std::env::var("MANUAL_AGENT_BASE_URL")
            .map_err(|_| "config: MANUAL_AGENT_BASE_URL required".to_string())?;
        let token = std::env::var("MANUAL_AGENT_TOKEN")
            .map_err(|_| "config: MANUAL_AGENT_TOKEN required".to_string())?;
        if token.trim().len() < 32 {
            return Err("config: MANUAL_AGENT_TOKEN must contain at least 32 characters".into());
        }
        let base_url = validate_base_url(&base_url)?;
        let poll_interval = Duration::from_millis(parse_env_range(
            "MANUAL_AGENT_POLL_INTERVAL_MS",
            2_000,
            250,
            30_000,
        )?);
        let run_timeout = Duration::from_secs(parse_env_range(
            "MANUAL_AGENT_RUN_TIMEOUT_SECS",
            3_600,
            60,
            14_400,
        )?);
        let model = non_blank_env("MANUAL_AGENT_MODEL");
        let reasoning_effort = non_blank_env("MANUAL_AGENT_REASONING_EFFORT");
        if let Some(effort) = reasoning_effort.as_deref() {
            if !matches!(effort, "minimal" | "low" | "medium" | "high" | "xhigh") {
                return Err(
                    "config: MANUAL_AGENT_REASONING_EFFORT must be minimal, low, medium, high, or xhigh"
                        .into(),
                );
            }
        }
        Ok(Self {
            base_url,
            token,
            poll_interval,
            run_timeout,
            model,
            reasoning_effort,
        })
    }
}

fn non_blank_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_env_range(key: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    let value = match std::env::var(key) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| format!("config: {key} must be an integer"))?,
        Err(_) => default,
    };
    if !(min..=max).contains(&value) {
        return Err(format!("config: {key} must be between {min} and {max}"));
    }
    Ok(value)
}

fn validate_base_url(raw: &str) -> Result<Url, String> {
    let mut url = Url::parse(raw.trim())
        .map_err(|error| format!("config: MANUAL_AGENT_BASE_URL is invalid: {error}"))?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "config: MANUAL_AGENT_BASE_URL cannot contain credentials, query, or fragment".into(),
        );
    }
    if !matches!(url.path(), "" | "/") {
        return Err("config: MANUAL_AGENT_BASE_URL cannot contain a path".into());
    }
    if url.scheme() != "https" || !url.host_str().is_some_and(is_tailnet_hostname) {
        return Err(
            "config: MANUAL_AGENT_BASE_URL must be a Tailnet HTTPS hostname ending in .ts.net"
                .into(),
        );
    }
    url.set_path("/");
    Ok(url)
}

fn is_tailnet_hostname(host: &str) -> bool {
    host.to_ascii_lowercase()
        .strip_suffix(".ts.net")
        .is_some_and(|prefix| !prefix.is_empty() && !prefix.ends_with('.'))
}

fn valid_computer_session_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=120).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_computer_session(
    descriptor: &mut ComputerSessionDescriptor,
    expected_endpoint: Option<&Url>,
) -> Result<(), String> {
    if descriptor.version != 1 || !valid_computer_session_id(&descriptor.session_id) {
        return Err("remote run returned an invalid computer session descriptor".into());
    }
    let mut endpoint = Url::parse(descriptor.endpoint.trim())
        .map_err(|_| "remote run returned an invalid computer session endpoint".to_string())?;
    if endpoint.scheme() != "https"
        || !endpoint.host_str().is_some_and(is_tailnet_hostname)
        || endpoint.username() != ""
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
        || !matches!(endpoint.path(), "" | "/")
    {
        return Err("remote run returned a non-Tailnet computer session endpoint".into());
    }
    if let Some(expected_endpoint) =
        expected_endpoint.filter(|url| url.host_str().is_some_and(is_tailnet_hostname))
    {
        if endpoint.origin() != expected_endpoint.origin() {
            return Err(
                "remote run returned a computer session for a different Tailnet gateway".into(),
            );
        }
    }
    endpoint.set_path("");
    descriptor.endpoint = endpoint.to_string().trim_end_matches('/').to_string();
    Ok(())
}

#[derive(Clone)]
struct RemoteClient {
    http: Client,
    config: Config,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ComputerSessionDescriptor {
    version: u8,
    session_id: String,
    endpoint: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RunStatus {
    run_id: String,
    status: String,
    #[serde(default)]
    message: Value,
    #[serde(default)]
    final_response: Option<String>,
    #[serde(default)]
    computer_session: Option<ComputerSessionDescriptor>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadinessAttestation {
    ok: bool,
    execution_backend: String,
    backend_ready: bool,
}

impl RemoteClient {
    fn new(config: Config) -> Result<Self, String> {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("buzz-manual-agent-acp/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("config: could not build HTTP client: {error}"))?;
        Ok(Self { http, config })
    }

    fn endpoint(&self, path: &str) -> Result<Url, String> {
        self.config
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| format!("remote URL error: {error}"))
    }

    fn sensitive_values(&self) -> [&str; 2] {
        [&self.config.token, self.config.base_url.as_str()]
    }

    async fn verify_ready(&self) -> Result<(), String> {
        let response = self
            .http
            .get(self.endpoint("runs?limit=1")?)
            .bearer_auth(&self.config.token)
            .send()
            .await
            .map_err(|error| transport_error("connectivity probe", &error))?;
        let runs: Value = parse_response(
            response,
            StatusCode::OK,
            "connectivity probe",
            &self.sensitive_values(),
        )
        .await?;
        if !runs.get("runs").is_some_and(Value::is_array) {
            return Err("remote connectivity probe returned an invalid contract".to_string());
        }

        let response = self
            .http
            .get(self.endpoint("readiness")?)
            .bearer_auth(&self.config.token)
            .send()
            .await
            .map_err(|error| transport_error("backend readiness probe", &error))?;
        let readiness: ReadinessAttestation = parse_response(
            response,
            StatusCode::OK,
            "backend readiness probe",
            &self.sensitive_values(),
        )
        .await?;
        if !readiness.ok
            || !readiness.backend_ready
            || readiness.execution_backend.trim() != "crabbox"
        {
            return Err(
                "remote execution backend is not attested ready; refusing to accept prompts"
                    .to_string(),
            );
        }
        Ok(())
    }

    async fn enqueue(&self, prompt: &str) -> Result<RunStatus, String> {
        let mut body = json!({
            "prompt": prompt,
            "primitive": "run",
            "mode": "one-shot",
            "source": { "kind": "empty" },
            "createDraftPr": false,
            "install": "none"
        });
        if let Some(model) = self.config.model.as_deref() {
            body["model"] = json!(model);
        }
        if let Some(effort) = self.config.reasoning_effort.as_deref() {
            body["modelReasoningEffort"] = json!(effort);
        }
        let response = self
            .http
            .post(self.endpoint("runs")?)
            .bearer_auth(&self.config.token)
            .json(&body)
            .send()
            .await
            .map_err(|error| transport_error("run submission", &error))?;
        parse_run_response(
            response,
            StatusCode::ACCEPTED,
            "run submission",
            &self.sensitive_values(),
            Some(&self.config.base_url),
        )
        .await
    }

    async fn status(&self, run_id: &str) -> Result<RunStatus, String> {
        let response = self
            .http
            .get(self.endpoint(&format!("runs/{run_id}"))?)
            .bearer_auth(&self.config.token)
            .send()
            .await
            .map_err(|error| transport_error("run status", &error))?;
        parse_run_response(
            response,
            StatusCode::OK,
            "run status",
            &self.sensitive_values(),
            Some(&self.config.base_url),
        )
        .await
    }

    async fn stop(&self, run_id: &str) -> Result<RunStatus, String> {
        let response = self
            .http
            .post(self.endpoint(&format!("runs/{run_id}/stop"))?)
            .bearer_auth(&self.config.token)
            .send()
            .await
            .map_err(|error| transport_error("run stop", &error))?;
        parse_run_response(
            response,
            StatusCode::OK,
            "run stop",
            &self.sensitive_values(),
            Some(&self.config.base_url),
        )
        .await
    }
}

fn transport_error(stage: &str, error: &reqwest::Error) -> String {
    let class = if error.is_timeout() {
        "timed out"
    } else if error.is_connect() {
        "could not connect"
    } else if error.is_request() {
        "request failed"
    } else {
        "transport failed"
    };
    // reqwest's Display includes the request URL. The URL is setup data and
    // must not be copied into ACP error messages or channel-visible frames.
    format!("remote {stage} {class}")
}

fn sanitize_error_text(raw: &str, sensitive_values: &[&str]) -> Option<String> {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return None;
    }
    let lower = compact.to_ascii_lowercase();
    let contains_secret_material = [
        "authorization:",
        "authorization=",
        "bearer ",
        "access_token=",
        "refresh_token=",
        "api_key=",
        "api-key=",
        "apikey=",
        "token=",
        "password=",
        "secret=",
        "sk-",
        "ghp_",
        "xoxb-",
        "xoxp-",
    ]
    .iter()
    .any(|marker| lower.contains(marker));
    let contains_configured_value = sensitive_values.iter().any(|sensitive| {
        let sensitive = sensitive.trim();
        !sensitive.is_empty()
            && (compact.contains(sensitive)
                || (sensitive.ends_with('/') && compact.contains(sensitive.trim_end_matches('/'))))
    });
    if contains_secret_material || contains_configured_value {
        return Some("remote error details redacted".to_string());
    }
    let mut bounded = compact
        .chars()
        .take(MAX_ERROR_DETAIL_CHARS)
        .collect::<String>();
    if compact.chars().count() > MAX_ERROR_DETAIL_CHARS {
        bounded.push('…');
    }
    Some(bounded)
}

fn sanitize_error_code(value: &Value) -> Option<String> {
    let raw = value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|number| number.to_string()))?;
    let code = raw
        .chars()
        .take(64)
        .filter(|character| character.is_ascii_alphanumeric() || "_.-".contains(*character))
        .collect::<String>();
    (!code.is_empty()).then_some(code)
}

fn sanitize_error_value(value: &Value, depth: usize, sensitive_values: &[&str]) -> Option<String> {
    if depth > 4 {
        return None;
    }
    match value {
        Value::String(text) => sanitize_error_text(text, sensitive_values),
        Value::Object(object) => {
            if let Some(nested) = object.get("error") {
                if let Some(detail) = sanitize_error_value(nested, depth + 1, sensitive_values) {
                    return Some(detail);
                }
            }
            let code = object.get("code").and_then(sanitize_error_code);
            let message = ["message", "detail", "reason", "cause"]
                .iter()
                .find_map(|key| object.get(*key))
                .and_then(|nested| sanitize_error_value(nested, depth + 1, sensitive_values));
            match (code, message) {
                (Some(code), Some(message)) => Some(format!("{code}: {message}")),
                (Some(code), None) => Some(code),
                (None, message) => message,
            }
        }
        _ => None,
    }
}

fn status_message(status: &RunStatus) -> Option<String> {
    sanitize_error_value(&status.message, 0, &[])
}

async fn parse_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    expected: StatusCode,
    stage: &str,
    sensitive_values: &[&str],
) -> Result<T, String> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| format!("remote {stage} response could not be read"))?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(format!("remote {stage} response exceeded the size limit"));
    }
    if status != expected {
        let detail = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|value| sanitize_error_value(&value, 0, sensitive_values));
        let category = match status {
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => "configuration rejected",
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => "authentication rejected",
            StatusCode::NOT_FOUND => "resource not found",
            StatusCode::CONFLICT => "request conflicted",
            StatusCode::TOO_MANY_REQUESTS => "rate limited",
            StatusCode::BAD_GATEWAY
            | StatusCode::SERVICE_UNAVAILABLE
            | StatusCode::GATEWAY_TIMEOUT => "provider unavailable",
            _ => "request failed",
        };
        return Err(match detail {
            Some(detail) => format!("remote {stage} {category} ({status}): {detail}"),
            None => format!("remote {stage} {category} ({status})"),
        });
    }
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("remote {stage} returned invalid JSON: {error}"))
}

async fn parse_run_response(
    response: reqwest::Response,
    expected: StatusCode,
    stage: &str,
    sensitive_values: &[&str],
    expected_computer_endpoint: Option<&Url>,
) -> Result<RunStatus, String> {
    let mut status: RunStatus = parse_response(response, expected, stage, sensitive_values).await?;
    status.message = sanitize_error_value(&status.message, 0, sensitive_values)
        .map(Value::String)
        .unwrap_or(Value::Null);
    if let Some(descriptor) = status.computer_session.as_mut() {
        validate_computer_session(descriptor, expected_computer_endpoint)?;
    }
    Ok(status)
}

#[derive(Clone)]
struct Session {
    system_prompt: String,
    busy: bool,
    cancelled: Arc<AtomicBool>,
    cancel_notify: Arc<Notify>,
    run_id: Option<String>,
}

type Sessions = Arc<Mutex<HashMap<String, Session>>>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitializeParams {
    protocol_version: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionNewParams {
    cwd: String,
    #[serde(default)]
    system_prompt: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionPromptParams {
    session_id: String,
    prompt: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionCancelParams {
    session_id: String,
}

type WireSender = mpsc::Sender<Value>;

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::from_env().map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async_main(
        RemoteClient::new(config).map_err(|error| error.to_string())?,
    ))?;
    Ok(())
}

async fn async_main(remote: RemoteClient) -> Result<(), String> {
    remote.verify_ready().await?;
    let sessions: Sessions = Arc::new(Mutex::new(HashMap::new()));
    let (wire_tx, mut wire_rx) = mpsc::channel::<Value>(64);
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(value) = wire_rx.recv().await {
            let Ok(mut encoded) = serde_json::to_vec(&value) else {
                continue;
            };
            encoded.push(b'\n');
            if stdout.write_all(&encoded).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > MAX_FRAME_BYTES {
            let _ = wire_tx
                .send(rpc_error(Value::Null, -32600, "request exceeds size limit"))
                .await;
            continue;
        }
        let message = match serde_json::from_str::<Value>(&line) {
            Ok(message) => message,
            Err(error) => {
                let _ = wire_tx
                    .send(rpc_error(
                        Value::Null,
                        -32700,
                        &format!("invalid JSON: {error}"),
                    ))
                    .await;
                continue;
            }
        };
        dispatch(message, sessions.clone(), remote.clone(), wire_tx.clone()).await;
    }
    drop(wire_tx);
    let _ = writer.await;
    Ok(())
}

async fn dispatch(message: Value, sessions: Sessions, remote: RemoteClient, wire: WireSender) {
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        let _ = wire
            .send(rpc_error(Value::Null, -32600, "invalid JSON-RPC version"))
            .await;
        return;
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return;
    };
    let id = message.get("id").cloned();
    let params = message.get("params").cloned().unwrap_or(Value::Null);

    match method {
        "initialize" => {
            let Some(id) = id else { return };
            let parsed: Result<InitializeParams, _> = serde_json::from_value(params);
            match parsed {
                Ok(params) => {
                    let _ = wire
                        .send(rpc_ok(
                            id,
                            json!({
                                "protocolVersion": params.protocol_version.min(PROTOCOL_VERSION),
                                "agentCapabilities": {
                                    "loadSession": false,
                                    "promptCapabilities": { "image": false, "audio": false, "embeddedContext": false },
                                    "mcpCapabilities": { "http": false, "sse": false }
                                },
                                "agentInfo": { "name": "pkzz-remote-agent", "version": env!("CARGO_PKG_VERSION") },
                                "_meta": { "pkzz": { "hostFinalReply": { "version": HOST_FINAL_REPLY_VERSION } } }
                            }),
                        ))
                        .await;
                }
                Err(error) => {
                    let _ = wire
                        .send(rpc_error(id, -32602, &format!("initialize: {error}")))
                        .await;
                }
            }
        }
        "session/new" => {
            let Some(id) = id else { return };
            let parsed: Result<SessionNewParams, _> = serde_json::from_value(params);
            match parsed {
                Ok(params) if !params.cwd.trim().is_empty() => {
                    let session_id = format!("remote_{}", uuid::Uuid::new_v4().simple());
                    sessions.lock().await.insert(
                        session_id.clone(),
                        Session {
                            system_prompt: params.system_prompt.unwrap_or_default(),
                            busy: false,
                            cancelled: Arc::new(AtomicBool::new(false)),
                            cancel_notify: Arc::new(Notify::new()),
                            run_id: None,
                        },
                    );
                    let _ = wire
                        .send(rpc_ok(id, json!({ "sessionId": session_id })))
                        .await;
                }
                Ok(_) => {
                    let _ = wire
                        .send(rpc_error(id, -32602, "session/new: cwd is required"))
                        .await;
                }
                Err(error) => {
                    let _ = wire
                        .send(rpc_error(id, -32602, &format!("session/new: {error}")))
                        .await;
                }
            }
        }
        "session/prompt" => {
            let Some(id) = id else { return };
            let parsed: Result<SessionPromptParams, _> = serde_json::from_value(params);
            match parsed {
                Ok(params) => {
                    let session = {
                        let mut guard = sessions.lock().await;
                        let Some(session) = guard.get_mut(&params.session_id) else {
                            let _ = wire
                                .send(rpc_error(id, -32602, "session/prompt: unknown session"))
                                .await;
                            return;
                        };
                        if session.busy {
                            let _ = wire
                                .send(rpc_error(id, -32602, "session/prompt: session is busy"))
                                .await;
                            return;
                        }
                        session.busy = true;
                        session.cancelled.store(false, Ordering::SeqCst);
                        session.cancel_notify = Arc::new(Notify::new());
                        session.clone()
                    };
                    let prompt = match build_prompt(&session.system_prompt, &params.prompt) {
                        Ok(prompt) => prompt,
                        Err(error) => {
                            if let Some(current) = sessions.lock().await.get_mut(&params.session_id)
                            {
                                current.busy = false;
                            }
                            let _ = wire.send(rpc_error(id, -32602, &error)).await;
                            return;
                        }
                    };
                    tokio::spawn(run_prompt(
                        id,
                        params.session_id,
                        prompt,
                        session.cancelled,
                        sessions,
                        remote,
                        wire,
                    ));
                }
                Err(error) => {
                    let _ = wire
                        .send(rpc_error(id, -32602, &format!("session/prompt: {error}")))
                        .await;
                }
            }
        }
        "session/cancel" => {
            let parsed: Result<SessionCancelParams, _> = serde_json::from_value(params);
            match parsed {
                Ok(params) => {
                    let result = cancel_session(&params.session_id, &sessions).await;
                    if let Some(id) = id {
                        let response = match result {
                            Ok(acknowledgement) => rpc_ok(id, acknowledgement),
                            Err(error) => rpc_error(id, -32000, &error),
                        };
                        let _ = wire.send(response).await;
                    }
                }
                Err(error) => {
                    if let Some(id) = id {
                        let _ = wire
                            .send(rpc_error(id, -32602, &format!("session/cancel: {error}")))
                            .await;
                    }
                }
            }
        }
        _ => {
            if let Some(id) = id {
                let _ = wire.send(rpc_error(id, -32601, "method not found")).await;
            }
        }
    }
}

fn build_prompt(system_prompt: &str, blocks: &[Value]) -> Result<String, String> {
    let mut parts = Vec::new();
    if !system_prompt.trim().is_empty() {
        parts.push(format!("[System]\n{}", system_prompt.trim()));
    }
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("text") {
            return Err("session/prompt: remote adapter accepts text blocks only".into());
        }
        let text = block
            .get("text")
            .and_then(Value::as_str)
            .ok_or_else(|| "session/prompt: text block is missing text".to_string())?;
        parts.push(text.to_string());
    }
    let prompt = parts.join("\n\n");
    if prompt.trim().is_empty() {
        return Err("session/prompt: prompt is empty".into());
    }
    if prompt.len() > MAX_PROMPT_BYTES {
        return Err("session/prompt: prompt exceeds 512 KiB".into());
    }
    Ok(prompt)
}

async fn cancel_session(session_id: &str, sessions: &Sessions) -> Result<Value, String> {
    let status = {
        let mut guard = sessions.lock().await;
        let Some(session) = guard.get_mut(session_id) else {
            return Err("session/cancel: unknown session".to_string());
        };
        session.cancelled.store(true, Ordering::SeqCst);
        session.cancel_notify.notify_one();
        if session.run_id.is_some() {
            "stop_requested"
        } else {
            "pending"
        }
    };
    Ok(json!({ "acknowledged": true, "status": status }))
}

async fn run_prompt(
    request_id: Value,
    session_id: String,
    prompt: String,
    cancelled: Arc<AtomicBool>,
    sessions: Sessions,
    remote: RemoteClient,
    wire: WireSender,
) {
    let outcome =
        run_remote_turn(&session_id, &prompt, &cancelled, &sessions, &remote, &wire).await;
    if let Some(session) = sessions.lock().await.get_mut(&session_id) {
        session.busy = false;
        session.run_id = None;
    }
    let response = match outcome {
        Ok(RemoteOutcome::Completed { content, .. }) => rpc_ok(
            request_id,
            json!({
                "stopReason": "end_turn",
                "_meta": { "pkzz": { "hostFinalReply": {
                    "version": HOST_FINAL_REPLY_VERSION,
                    "deliveryId": "final",
                    "content": content
                } } }
            }),
        ),
        Ok(RemoteOutcome::Cancelled) => rpc_ok(request_id, json!({ "stopReason": "cancelled" })),
        // A remote computer turn is side-effectful. Once submission is attempted,
        // the outer queue must never replay the same user prompt as a fresh run.
        Err(error) => rpc_error(request_id, NON_RETRYABLE_REMOTE_TURN_CODE, &error),
    };
    let _ = wire.send(response).await;
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RemoteOutcome {
    Completed {
        content: String,
        computer_session: Option<ComputerSessionDescriptor>,
    },
    Cancelled,
}

fn classify_remote_status(status: RunStatus) -> Option<Result<RemoteOutcome, String>> {
    let state = status.status.trim().to_ascii_lowercase();
    match state.as_str() {
        "completed" => Some(
            status
                .final_response
                .filter(|content| !content.trim().is_empty())
                .map(|content| RemoteOutcome::Completed {
                    content,
                    computer_session: status.computer_session,
                })
                .ok_or_else(|| "remote run completed without a final response".to_string()),
        ),
        "stopped" | "cancelled" => Some(Ok(RemoteOutcome::Cancelled)),
        "failed" if status_message(&status).as_deref() == Some(STOP_ACKNOWLEDGEMENT) => {
            Some(Ok(RemoteOutcome::Cancelled))
        }
        "failed"
        | "error"
        | "rejected"
        | "unauthorized"
        | "forbidden"
        | "configuration_error"
        | "config_error"
        | "provider_unavailable" => Some(Err(
            status_message(&status).unwrap_or_else(|| "remote run failed".to_string())
        )),
        _ => None,
    }
}

async fn request_remote_stop(remote: &RemoteClient, run_id: &str) -> Result<RemoteOutcome, String> {
    let status = remote.stop(run_id).await?;
    classify_remote_status(status)
        .unwrap_or_else(|| Err("remote stop was not acknowledged".to_string()))
}

async fn run_remote_turn(
    session_id: &str,
    prompt: &str,
    cancelled: &AtomicBool,
    sessions: &Sessions,
    remote: &RemoteClient,
    wire: &WireSender,
) -> Result<RemoteOutcome, String> {
    if cancelled.load(Ordering::SeqCst) {
        return Ok(RemoteOutcome::Cancelled);
    }
    let submitted = remote.enqueue(prompt).await?;
    let tool_call_id = format!("remote-{}", submitted.run_id);
    let _ = wire
        .send(session_update(
            session_id,
            json!({
                "sessionUpdate": "tool_call",
                "toolCallId": tool_call_id,
                "title": "Remote agent computer",
                "kind": "other",
                "status": "in_progress"
            }),
        ))
        .await;

    let outcome = run_enqueued_turn(session_id, submitted, cancelled, sessions, remote, wire).await;
    let mut terminal_update = match &outcome {
        Ok(RemoteOutcome::Completed {
            computer_session, ..
        }) => match computer_session {
            Some(descriptor) => json!({
                "status": "completed",
                "rawOutput": { "computerSession": descriptor }
            }),
            None => json!({ "status": "completed" }),
        },
        Ok(RemoteOutcome::Cancelled) => json!({
            "status": "failed",
            "content": [{
                "type": "content",
                "content": { "type": "text", "text": "Cancelled by request." }
            }],
            "rawOutput": { "cancelled": true }
        }),
        Err(_) => json!({ "status": "failed" }),
    };
    if let Some(update) = terminal_update.as_object_mut() {
        update.insert("sessionUpdate".to_string(), json!("tool_call_update"));
        update.insert("toolCallId".to_string(), json!(tool_call_id));
    }
    let _ = wire.send(session_update(session_id, terminal_update)).await;
    outcome
}

async fn run_enqueued_turn(
    session_id: &str,
    submitted: RunStatus,
    cancelled: &AtomicBool,
    sessions: &Sessions,
    remote: &RemoteClient,
    wire: &WireSender,
) -> Result<RemoteOutcome, String> {
    let cancel_notify = {
        let mut guard = sessions.lock().await;
        let session = guard
            .get_mut(session_id)
            .ok_or_else(|| "remote session disappeared".to_string())?;
        session.run_id = Some(submitted.run_id.clone());
        session.cancel_notify.clone()
    };

    let deadline = tokio::time::Instant::now() + remote.config.run_timeout;
    let mut next_progress = tokio::time::Instant::now() + PROGRESS_INTERVAL;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return request_remote_stop(remote, &submitted.run_id).await;
        }
        if tokio::time::Instant::now() >= deadline {
            return match request_remote_stop(remote, &submitted.run_id).await {
                Ok(completed @ RemoteOutcome::Completed { .. }) => Ok(completed),
                Ok(RemoteOutcome::Cancelled) => {
                    Err("remote run exceeded MANUAL_AGENT_RUN_TIMEOUT_SECS".into())
                }
                Err(stop_error) => Err(format!(
                    "remote run exceeded MANUAL_AGENT_RUN_TIMEOUT_SECS; {stop_error}"
                )),
            };
        }
        tokio::select! {
            _ = cancel_notify.notified() => continue,
            _ = tokio::time::sleep(remote.config.poll_interval) => {}
        }
        let status = match remote.status(&submitted.run_id).await {
            Ok(status) => status,
            Err(error) if is_transient_poll_error(&error) => continue,
            Err(error) => {
                return match request_remote_stop(remote, &submitted.run_id).await {
                    Ok(completed @ RemoteOutcome::Completed { .. }) => Ok(completed),
                    Ok(RemoteOutcome::Cancelled) => Err(error),
                    Err(stop_error) => Err(format!("{error}; cleanup stop failed: {stop_error}")),
                };
            }
        };
        if let Some(outcome) = classify_remote_status(status) {
            return outcome;
        }
        if tokio::time::Instant::now() >= next_progress {
            let _ = wire
                .send(session_update(
                    session_id,
                    json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": format!("remote-{}", submitted.run_id),
                        "status": "in_progress"
                    }),
                ))
                .await;
            next_progress = tokio::time::Instant::now() + PROGRESS_INTERVAL;
        }
    }
}

fn is_transient_poll_error(error: &str) -> bool {
    [
        "timed out",
        "could not connect",
        "transport failed",
        "rate limited",
        "provider unavailable",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn session_update(session_id: &str, update: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/update",
        "params": { "sessionId": session_id, "update": update }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::VecDeque,
        sync::{Arc as StdArc, Mutex as StdMutex},
    };

    use tokio::{io::AsyncReadExt, net::TcpListener};

    const TEST_TOKEN: &str = "test-app-password-0123456789-abcdef";

    #[derive(Clone)]
    struct MockReply {
        status: u16,
        body: Value,
    }

    async fn mock_remote(
        replies: Vec<MockReply>,
    ) -> (
        Url,
        StdArc<StdMutex<Vec<String>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock remote");
        let address = listener.local_addr().expect("mock address");
        let requests = StdArc::new(StdMutex::new(Vec::new()));
        let captured = requests.clone();
        let handle = tokio::spawn(async move {
            let mut replies = VecDeque::from(replies);
            while let Some(reply) = replies.pop_front() {
                let (mut socket, _) = listener.accept().await.expect("accept mock request");
                let mut request = Vec::new();
                let mut chunk = [0_u8; 4096];
                let header_end = loop {
                    let count = socket.read(&mut chunk).await.expect("read mock request");
                    assert!(count > 0, "request closed before headers");
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(position) =
                        request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                    {
                        break position + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                while request.len() < header_end + content_length {
                    let count = socket.read(&mut chunk).await.expect("read mock body");
                    assert!(count > 0, "request closed before body");
                    request.extend_from_slice(&chunk[..count]);
                }
                captured
                    .lock()
                    .expect("capture requests")
                    .push(String::from_utf8_lossy(&request).to_string());

                let body = serde_json::to_vec(&reply.body).expect("encode mock body");
                let reason = match reply.status {
                    200 => "OK",
                    202 => "Accepted",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    500 => "Internal Server Error",
                    503 => "Service Unavailable",
                    _ => "Test",
                };
                let response = format!(
                    "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    reply.status,
                    reason,
                    body.len()
                );
                socket
                    .write_all(response.as_bytes())
                    .await
                    .expect("write mock headers");
                socket.write_all(&body).await.expect("write mock body");
            }
        });
        (
            Url::parse(&format!("http://{address}/")).expect("mock URL"),
            requests,
            handle,
        )
    }

    fn test_remote(base_url: Url, poll_interval: Duration) -> RemoteClient {
        RemoteClient::new(Config {
            base_url,
            token: TEST_TOKEN.to_string(),
            poll_interval,
            run_timeout: Duration::from_secs(2),
            model: None,
            reasoning_effort: None,
        })
        .expect("test remote client")
    }

    fn test_sessions(cancelled: Arc<AtomicBool>) -> Sessions {
        Arc::new(Mutex::new(HashMap::from([(
            "session-1".to_string(),
            Session {
                system_prompt: String::new(),
                busy: true,
                cancelled,
                cancel_notify: Arc::new(Notify::new()),
                run_id: None,
            },
        )])))
    }

    fn update_status(frame: &Value) -> Option<&str> {
        frame
            .pointer("/params/update/status")
            .and_then(Value::as_str)
    }

    #[test]
    fn accepts_only_tailnet_https_before_any_request_is_built() {
        assert!(validate_base_url("https://device.example-tailnet.ts.net:8443").is_ok());
        assert!(validate_base_url("http://device.example-tailnet.ts.net:8443").is_err());
        assert!(validate_base_url("http://100.100.100.100:8443").is_err());
        assert!(validate_base_url("http://127.0.0.1:8443").is_err());
        assert!(validate_base_url("https://example.com").is_err());
        assert!(validate_base_url("https://ts.net").is_err());
    }

    #[test]
    fn base_url_rejects_credentials_and_paths() {
        assert!(validate_base_url("https://user:pass@device.tailnet.ts.net").is_err());
        assert!(validate_base_url("https://device.tailnet.ts.net/api").is_err());
    }

    #[test]
    fn computer_session_descriptor_is_minimal_and_tailnet_only() {
        let mut valid = ComputerSessionDescriptor {
            version: 1,
            session_id: "mesh-session_1".to_string(),
            endpoint: "https://mac.example-tailnet.ts.net:8443/".to_string(),
        };
        let expected = Url::parse("https://mac.example-tailnet.ts.net:8443").unwrap();
        validate_computer_session(&mut valid, Some(&expected)).expect("safe descriptor");
        assert_eq!(valid.endpoint, "https://mac.example-tailnet.ts.net:8443");

        for endpoint in [
            "https://public.example.com",
            "http://mac.example-tailnet.ts.net:8443",
            "https://user:secret@mac.example-tailnet.ts.net:8443",
            "https://mac.example-tailnet.ts.net:8443/path",
        ] {
            let mut descriptor = ComputerSessionDescriptor {
                version: 1,
                session_id: "mesh-session_1".to_string(),
                endpoint: endpoint.to_string(),
            };
            assert!(
                validate_computer_session(&mut descriptor, Some(&expected)).is_err(),
                "{endpoint}"
            );
        }

        for endpoint in [
            "https://other.example-tailnet.ts.net:8443",
            "https://mac.example-tailnet.ts.net:8787",
        ] {
            let mut wrong_gateway = ComputerSessionDescriptor {
                version: 1,
                session_id: "mesh-session_1".to_string(),
                endpoint: endpoint.to_string(),
            };
            assert!(validate_computer_session(&mut wrong_gateway, Some(&expected)).is_err());
        }
    }

    #[test]
    fn sanitizer_redacts_exact_configured_token_and_base_url_echoes() {
        let base_url = "https://device.example-tailnet.ts.net:8443/";
        let raw = json!({
            "error": {
                "message": format!("upstream echoed {TEST_TOKEN} from {base_url}")
            }
        });
        let sanitized =
            sanitize_error_value(&raw, 0, &[TEST_TOKEN, base_url]).expect("sanitized detail");
        assert_eq!(sanitized, "remote error details redacted");
        assert!(!sanitized.contains(TEST_TOKEN));
        assert!(!sanitized.contains(base_url));
    }

    #[tokio::test]
    async fn readiness_requires_authenticated_connectivity_and_backend_attestation() {
        let (base_url, requests, server) = mock_remote(vec![
            MockReply {
                status: 200,
                body: json!({ "runs": [] }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "ok": true,
                    "executionBackend": "crabbox",
                    "backendReady": true
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(1));
        remote.verify_ready().await.expect("attested backend ready");
        server.await.expect("mock server completes");
        let requests = requests.lock().expect("requests");
        assert!(requests[0].starts_with("GET /runs?limit=1 HTTP/1.1"));
        assert!(requests[1].starts_with("GET /readiness HTTP/1.1"));
        assert!(requests
            .iter()
            .all(|request| request.contains(&format!("authorization: Bearer {TEST_TOKEN}"))));
    }

    #[tokio::test]
    async fn readiness_fails_closed_without_execution_backend_attestation() {
        let (base_url, _, server) = mock_remote(vec![
            MockReply {
                status: 200,
                body: json!({ "runs": [] }),
            },
            MockReply {
                status: 404,
                body: json!({ "error": "readiness contract unavailable" }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(1));
        let error = remote
            .verify_ready()
            .await
            .expect_err("missing attestation must fail closed");
        assert!(error.contains("backend readiness probe"));
        assert!(!error.contains(TEST_TOKEN));
        server.await.expect("mock server completes");
    }

    #[tokio::test]
    async fn readiness_rejects_host_execution_even_when_backend_claims_ready() {
        let (base_url, _, server) = mock_remote(vec![
            MockReply {
                status: 200,
                body: json!({ "runs": [] }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "ok": true,
                    "executionBackend": "host",
                    "backendReady": true
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(1));
        let error = remote
            .verify_ready()
            .await
            .expect_err("host execution must not satisfy isolated readiness");
        assert!(error.contains("not attested ready"));
        server.await.expect("mock server completes");
    }

    #[test]
    fn prompt_combines_system_and_text_blocks() {
        let prompt = build_prompt(
            "Be precise.",
            &[json!({ "type": "text", "text": "Do the work." })],
        )
        .expect("valid text prompt");
        assert_eq!(prompt, "[System]\nBe precise.\n\nDo the work.");
    }

    #[test]
    fn prompt_rejects_non_text_blocks() {
        assert!(build_prompt("", &[json!({ "type": "image", "data": "x" })]).is_err());
    }

    #[tokio::test]
    async fn completed_turn_preserves_semantic_final_and_emits_terminal_update() {
        let (base_url, requests, server) = mock_remote(vec![
            MockReply {
                status: 202,
                body: json!({ "runId": "run-1", "status": "queued" }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "runId": "run-1",
                    "status": "completed",
                    "message": "done",
                    "finalResponse": "## Result\n\nSemantic answer.",
                    "computerSession": {
                        "version": 1,
                        "sessionId": "mesh-session-1",
                        "endpoint": "https://mac.example-tailnet.ts.net:8443"
                    }
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url.clone(), Duration::from_millis(1));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sessions = test_sessions(cancelled.clone());
        let (wire, mut updates) = mpsc::channel(8);

        let outcome = run_remote_turn(
            "session-1",
            "Do the work",
            &cancelled,
            &sessions,
            &remote,
            &wire,
        )
        .await
        .expect("remote turn completes");
        assert_eq!(
            outcome,
            RemoteOutcome::Completed {
                content: "## Result\n\nSemantic answer.".to_string(),
                computer_session: Some(ComputerSessionDescriptor {
                    version: 1,
                    session_id: "mesh-session-1".to_string(),
                    endpoint: "https://mac.example-tailnet.ts.net:8443".to_string(),
                }),
            }
        );
        let started = updates.recv().await.expect("tool call start");
        let terminal = updates.recv().await.expect("tool call completion");
        assert_eq!(update_status(&terminal), Some("completed"));
        assert_eq!(
            terminal.pointer("/params/update/rawOutput/computerSession/sessionId"),
            Some(&json!("mesh-session-1"))
        );

        let visible_frames = format!("{started}{terminal}");
        assert!(!visible_frames.contains(TEST_TOKEN));
        assert!(!visible_frames.contains(base_url.as_str()));
        server.await.expect("mock server completes");
        assert_eq!(requests.lock().expect("requests").len(), 2);
    }

    #[tokio::test]
    async fn nested_provider_failure_is_sanitized_and_emits_failed_update() {
        let leaked = "provider-token=super-secret-value";
        let (base_url, _, server) = mock_remote(vec![
            MockReply {
                status: 202,
                body: json!({ "runId": "run-2", "status": "queued" }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "runId": "run-2",
                    "status": "failed",
                    "message": {
                        "error": {
                            "code": "provider_unavailable",
                            "message": leaked,
                            "token": "super-secret-value"
                        }
                    }
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(1));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sessions = test_sessions(cancelled.clone());
        let (wire, mut updates) = mpsc::channel(8);

        let error = run_remote_turn(
            "session-1",
            "Do the work",
            &cancelled,
            &sessions,
            &remote,
            &wire,
        )
        .await
        .expect_err("provider failure must terminate the turn");
        assert!(error.contains("provider_unavailable"));
        assert!(error.contains("redacted"));
        assert!(!error.contains(leaked));
        assert!(!error.contains("super-secret-value"));
        let _started = updates.recv().await.expect("tool call start");
        let terminal = updates.recv().await.expect("tool call failure");
        assert_eq!(update_status(&terminal), Some("failed"));
        server.await.expect("mock server completes");
    }

    #[tokio::test]
    async fn cancellation_requires_stop_ack_and_emits_cancelled_update() {
        let (base_url, requests, server) = mock_remote(vec![
            MockReply {
                status: 202,
                body: json!({ "runId": "run-3", "status": "queued" }),
            },
            MockReply {
                status: 200,
                body: json!({ "runId": "run-3", "status": "running" }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "runId": "run-3",
                    "status": "failed",
                    "message": STOP_ACKNOWLEDGEMENT
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(10));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sessions = test_sessions(cancelled.clone());
        let (wire, mut updates) = mpsc::channel(8);

        let turn = run_remote_turn(
            "session-1",
            "Do the work",
            &cancelled,
            &sessions,
            &remote,
            &wire,
        );
        let request_cancel = async {
            let started = updates.recv().await.expect("tool call start");
            cancelled.store(true, Ordering::SeqCst);
            started
        };
        let (outcome, _started) = tokio::join!(turn, request_cancel);
        assert_eq!(
            outcome.expect("acknowledged cancellation"),
            RemoteOutcome::Cancelled
        );
        let terminal = updates.recv().await.expect("tool call cancellation");
        assert_eq!(update_status(&terminal), Some("failed"));
        assert_eq!(
            terminal.pointer("/params/update/rawOutput/cancelled"),
            Some(&Value::Bool(true))
        );
        assert_eq!(
            terminal.pointer("/params/update/content/0/content/text"),
            Some(&Value::String("Cancelled by request.".to_string()))
        );
        server.await.expect("mock server completes");
        let requests = requests.lock().expect("requests");
        assert!(requests
            .iter()
            .any(|request| request.starts_with("POST /runs/run-3/stop ")));
    }

    #[tokio::test]
    async fn stop_failure_is_surfaced_and_turn_is_marked_failed() {
        let leaked = "token=do-not-copy-this";
        let (base_url, _, server) = mock_remote(vec![
            MockReply {
                status: 202,
                body: json!({ "runId": "run-4", "status": "queued" }),
            },
            MockReply {
                status: 200,
                body: json!({ "runId": "run-4", "status": "running" }),
            },
            MockReply {
                status: 500,
                body: json!({ "error": { "message": leaked } }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(10));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sessions = test_sessions(cancelled.clone());
        let (wire, mut updates) = mpsc::channel(8);

        let turn = run_remote_turn(
            "session-1",
            "Do the work",
            &cancelled,
            &sessions,
            &remote,
            &wire,
        );
        let request_cancel = async {
            let started = updates.recv().await.expect("tool call start");
            cancelled.store(true, Ordering::SeqCst);
            started
        };
        let (outcome, _started) = tokio::join!(turn, request_cancel);
        let error = outcome.expect_err("unacknowledged stop must fail the turn");
        assert!(error.contains("run stop"));
        assert!(error.contains("redacted"));
        assert!(!error.contains(leaked));
        let terminal = updates.recv().await.expect("tool call failure");
        assert_eq!(update_status(&terminal), Some("failed"));
        server.await.expect("mock server completes");
    }

    #[tokio::test]
    async fn idless_cancel_wakes_prompt_worker_for_exactly_one_stop_without_rpc_response() {
        let (base_url, requests, server) = mock_remote(vec![
            MockReply {
                status: 202,
                body: json!({ "runId": "run-cancel", "status": "queued" }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "runId": "run-cancel",
                    "status": "failed",
                    "message": STOP_ACKNOWLEDGEMENT
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_secs(30));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sessions = test_sessions(cancelled.clone());
        let (tool_wire, mut tool_updates) = mpsc::channel(4);
        let (cancel_wire, mut cancel_frames) = mpsc::channel(4);
        let turn = run_remote_turn(
            "session-1",
            "Do the work",
            &cancelled,
            &sessions,
            &remote,
            &tool_wire,
        );
        let cancel = async {
            let started = tool_updates.recv().await.expect("tool call start");
            dispatch(
                json!({
                    "jsonrpc": "2.0",
                    "method": "session/cancel",
                    "params": { "sessionId": "session-1" }
                }),
                sessions.clone(),
                remote.clone(),
                cancel_wire,
            )
            .await;
            started
        };
        let (outcome, _started) = tokio::time::timeout(Duration::from_millis(500), async {
            tokio::join!(turn, cancel)
        })
        .await
        .expect("cancel must wake the worker without waiting for poll sleep");
        assert_eq!(
            outcome.expect("stop acknowledged"),
            RemoteOutcome::Cancelled
        );
        let terminal = tool_updates.recv().await.expect("terminal update");
        assert_eq!(update_status(&terminal), Some("failed"));
        server.await.expect("mock server completes");

        assert!(cancelled.load(Ordering::SeqCst));
        let captured = requests.lock().expect("captured requests");
        assert_eq!(captured.len(), 2);
        assert_eq!(
            captured
                .iter()
                .filter(|request| request.starts_with("POST /runs HTTP/1.1"))
                .count(),
            1
        );
        assert_eq!(
            captured
                .iter()
                .filter(|request| request.starts_with("POST /runs/run-cancel/stop HTTP/1.1"))
                .count(),
            1
        );
        assert!(matches!(
            cancel_frames.try_recv(),
            Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn deterministic_auth_error_exits_after_one_status_request() {
        let (base_url, requests, server) = mock_remote(vec![
            MockReply {
                status: 202,
                body: json!({ "runId": "run-5", "status": "queued" }),
            },
            MockReply {
                status: 401,
                body: json!({ "error": "Unauthorized" }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "runId": "run-5",
                    "status": "failed",
                    "message": STOP_ACKNOWLEDGEMENT
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(1));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sessions = test_sessions(cancelled.clone());
        let (wire, mut updates) = mpsc::channel(8);

        let error = run_remote_turn(
            "session-1",
            "Do the work",
            &cancelled,
            &sessions,
            &remote,
            &wire,
        )
        .await
        .expect_err("auth rejection must terminate immediately");
        assert!(error.contains("authentication rejected"));
        let _started = updates.recv().await.expect("tool call start");
        assert_eq!(
            update_status(&updates.recv().await.expect("terminal failure")),
            Some("failed")
        );
        server.await.expect("mock server completes");
        let requests = requests.lock().expect("requests");
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST /runs HTTP/1.1"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn transient_status_failure_retries_in_place_without_duplicate_submission() {
        let (base_url, requests, server) = mock_remote(vec![
            MockReply {
                status: 202,
                body: json!({ "runId": "run-retry", "status": "queued" }),
            },
            MockReply {
                status: 503,
                body: json!({ "error": "temporarily unavailable" }),
            },
            MockReply {
                status: 200,
                body: json!({
                    "runId": "run-retry",
                    "status": "completed",
                    "finalResponse": "Completed once."
                }),
            },
        ])
        .await;
        let remote = test_remote(base_url, Duration::from_millis(1));
        let cancelled = Arc::new(AtomicBool::new(false));
        let sessions = test_sessions(cancelled.clone());
        let (wire, mut updates) = mpsc::channel(8);

        let outcome = run_remote_turn(
            "session-1",
            "Do the work",
            &cancelled,
            &sessions,
            &remote,
            &wire,
        )
        .await
        .expect("transient status error should recover in place");
        assert_eq!(
            outcome,
            RemoteOutcome::Completed {
                content: "Completed once.".to_string(),
                computer_session: None,
            }
        );
        let _started = updates.recv().await.expect("tool call start");
        assert_eq!(
            update_status(&updates.recv().await.expect("terminal update")),
            Some("completed")
        );
        server.await.expect("mock server completes");
        let requests = requests.lock().expect("requests");
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST /runs HTTP/1.1"))
                .count(),
            1
        );
    }
}
