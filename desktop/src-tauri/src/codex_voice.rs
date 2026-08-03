use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    time::Duration,
};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use crate::managed_agents::{
    known_acp_runtime, load_managed_agents, load_personas, record_agent_command, resolve_command,
    KnownAcpRuntime,
};

const LINKS_FILE_NAME: &str = "codex-voice-links.json";
const VOICE_EVENT: &str = "codex-voice-event";
const REALTIME_MODEL: &str = "gpt-live-1-codex";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct CodexVoiceState {
    sessions: HashMap<String, CodexVoiceSession>,
}

struct CodexVoiceSession {
    thread_id: String,
    runtime_thread_id: String,
    muted: bool,
    mode: CodexVoiceMode,
    voice: String,
    client: CodexAppServerClient,
}

struct CodexAppServerClient {
    child: Child,
    stdin: Arc<Mutex<ChildStdin>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>>,
    expected_shutdown: Arc<AtomicBool>,
}

struct CodexVoiceRuntimeEnv {
    private_key_nsec: String,
    auth_tag: Option<String>,
    relay_url: String,
    mode: CodexVoiceMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodexVoiceMode {
    Native,
    Proxy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexVoiceTargetLink {
    channel_id: String,
    thread_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexVoiceEvent {
    method: String,
    params: Value,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexVoiceCapability {
    supported: bool,
    reason: Option<String>,
    model: Option<String>,
    mode: Option<CodexVoiceMode>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexVoiceStartResponse {
    muted: bool,
    model: String,
    mode: CodexVoiceMode,
    voice: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexVoiceStatus {
    active: bool,
    muted: bool,
    model: Option<String>,
    sessions: Vec<CodexVoiceSessionStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexVoiceSessionStatus {
    thread_id: String,
    muted: bool,
    model: String,
    mode: CodexVoiceMode,
    voice: String,
}

fn agent_voice_mode(
    app: &AppHandle,
    pubkey: &str,
    _relay_url: &str,
) -> Result<Option<CodexVoiceMode>, String> {
    let records = load_managed_agents(app)?;
    let personas = load_personas(app).unwrap_or_default();
    let Some(record) = records
        .iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(pubkey))
    else {
        return Ok(None);
    };
    let command = record_agent_command(record, &personas);
    Ok(Some(resolve_voice_mode(
        &record.name,
        known_acp_runtime(&command).is_some_and(KnownAcpRuntime::supports_native_voice),
    )))
}

fn resolve_voice_mode(_agent_name: &str, _supports_native_voice: bool) -> CodexVoiceMode {
    CodexVoiceMode::Proxy
}

fn codex_voice_runtime_env(
    app: &AppHandle,
    pubkey: &str,
    relay_url: &str,
) -> Result<Option<CodexVoiceRuntimeEnv>, String> {
    let records = load_managed_agents(app)?;
    let personas = load_personas(app).unwrap_or_default();
    let Some(record) = records
        .into_iter()
        .find(|record| record.pubkey.eq_ignore_ascii_case(pubkey))
    else {
        return Ok(None);
    };
    let command = record_agent_command(&record, &personas);
    let mode = resolve_voice_mode(
        &record.name,
        known_acp_runtime(&command).is_some_and(KnownAcpRuntime::supports_native_voice),
    );
    if record.private_key_nsec.is_empty() {
        return Err("The managed agent identity is unavailable.".to_string());
    }
    let workspace_relay =
        crate::relay::relay_ws_url_with_override(&app.state::<crate::app_state::AppState>());
    let relay_url = resolve_voice_relay_url(&record.relay_url, relay_url, &workspace_relay);
    if relay_url.is_empty() {
        return Err("The managed agent relay is unavailable.".to_string());
    }
    Ok(Some(CodexVoiceRuntimeEnv {
        private_key_nsec: record.private_key_nsec,
        auth_tag: record.auth_tag,
        relay_url,
        mode,
    }))
}

fn resolve_voice_relay_url(
    record_relay_url: &str,
    requested_relay_url: &str,
    workspace_relay_url: &str,
) -> String {
    let requested_relay_url = requested_relay_url.trim();
    let candidate = if requested_relay_url.is_empty() {
        record_relay_url
    } else {
        requested_relay_url
    };
    crate::relay::effective_agent_relay_url(candidate, workspace_relay_url)
}

fn configure_voice_runtime_env(command: &mut Command, runtime_env: &CodexVoiceRuntimeEnv) {
    command.env("BUZZ_PRIVATE_KEY", &runtime_env.private_key_nsec);
    command.env("NOSTR_PRIVATE_KEY", &runtime_env.private_key_nsec);
    command.env("BUZZ_RELAY_URL", &runtime_env.relay_url);
    if let Some(auth_tag) = &runtime_env.auth_tag {
        command.env("BUZZ_AUTH_TAG", auth_tag);
    } else {
        command.env_remove("BUZZ_AUTH_TAG");
    }
}

fn realtime_voice(requested: &str) -> &'static str {
    match requested {
        "arbor" => "arbor",
        "breeze" => "breeze",
        "cove" => "cove",
        "ember" => "ember",
        "juniper" => "juniper",
        "maple" => "maple",
        "spruce" => "spruce",
        "vale" => "vale",
        _ => "sol",
    }
}

fn voice_links_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|error| format!("Could not resolve Buzz app data: {error}"))?
        .join("agents")
        .join(LINKS_FILE_NAME))
}

fn voice_link_key(pubkey: &str, channel_id: &str) -> String {
    format!("{}:{channel_id}", pubkey.to_ascii_lowercase())
}

fn agent_voice_link_key(pubkey: &str) -> String {
    voice_link_key(pubkey, "*")
}

fn agent_voice_target_key(pubkey: &str) -> String {
    format!("target:{}", pubkey.to_ascii_lowercase())
}

fn load_voice_links(app: &AppHandle) -> Result<HashMap<String, String>, String> {
    let path = voice_links_path(app)?;
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes =
        fs::read(&path).map_err(|error| format!("Could not read Codex Voice links: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("Could not parse Codex Voice links: {error}"))
}

#[tauri::command]
pub fn get_codex_voice_link(
    pubkey: String,
    channel_id: String,
    app: AppHandle,
) -> Result<Option<String>, String> {
    let links = load_voice_links(&app)?;
    Ok(links
        .get(&voice_link_key(&pubkey, &channel_id))
        .or_else(|| links.get(&agent_voice_link_key(&pubkey)))
        .or_else(|| {
            let prefix = format!("{}:", pubkey.to_ascii_lowercase());
            links
                .iter()
                .find_map(|(key, thread_id)| key.starts_with(&prefix).then_some(thread_id))
        })
        .cloned())
}

#[tauri::command]
pub fn get_codex_voice_target_link(
    pubkey: String,
    app: AppHandle,
) -> Result<Option<CodexVoiceTargetLink>, String> {
    let links = load_voice_links(&app)?;
    if let Some(encoded) = links.get(&agent_voice_target_key(&pubkey)) {
        if let Some((channel_id, thread_id)) = encoded.split_once('\u{001f}') {
            return Ok(Some(CodexVoiceTargetLink {
                channel_id: channel_id.to_string(),
                thread_id: thread_id.to_string(),
            }));
        }
    }

    let prefix = format!("{}:", pubkey.to_ascii_lowercase());
    Ok(links.iter().find_map(|(key, thread_id)| {
        let channel_id = key.strip_prefix(&prefix)?;
        if channel_id == "*" || channel_id.is_empty() {
            return None;
        }
        Some(CodexVoiceTargetLink {
            channel_id: channel_id.to_string(),
            thread_id: thread_id.clone(),
        })
    }))
}

#[tauri::command]
pub fn remember_codex_voice_link(
    pubkey: String,
    channel_id: String,
    thread_id: String,
    app: AppHandle,
) -> Result<(), String> {
    if pubkey.is_empty() || channel_id.is_empty() || thread_id.is_empty() {
        return Err("Codex Voice link fields cannot be empty.".to_string());
    }
    let path = voice_links_path(&app)?;
    let parent = path
        .parent()
        .ok_or_else(|| "Codex Voice link path has no parent.".to_string())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create Codex Voice link directory: {error}"))?;
    let mut links = load_voice_links(&app)?;
    links.insert(voice_link_key(&pubkey, &channel_id), thread_id.clone());
    links.insert(agent_voice_link_key(&pubkey), thread_id.clone());
    links.insert(
        agent_voice_target_key(&pubkey),
        format!("{channel_id}\u{001f}{thread_id}"),
    );
    let bytes = serde_json::to_vec_pretty(&links)
        .map_err(|error| format!("Could not encode Codex Voice links: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Could not write Codex Voice links: {error}"))?;
    fs::rename(&temporary, &path)
        .map_err(|error| format!("Could not save Codex Voice links: {error}"))
}

#[tauri::command]
pub fn get_codex_voice_capability(
    pubkey: String,
    relay_url: String,
    app: AppHandle,
) -> Result<CodexVoiceCapability, String> {
    let Some(mode) = agent_voice_mode(&app, &pubkey, &relay_url)? else {
        return Ok(CodexVoiceCapability {
            supported: false,
            reason: None,
            model: None,
            mode: None,
        });
    };
    if resolve_command("codex").is_none() {
        return Ok(CodexVoiceCapability {
            supported: false,
            reason: Some("The Codex runtime is not installed.".to_string()),
            model: None,
            mode: None,
        });
    }
    Ok(CodexVoiceCapability {
        supported: true,
        reason: None,
        model: Some(REALTIME_MODEL.to_string()),
        mode: Some(mode),
    })
}

#[cfg(target_os = "macos")]
#[tauri::command]
pub fn request_microphone_access() -> Result<bool, String> {
    use block2::RcBlock;
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    let audio_media_type = unsafe { AVMediaTypeAudio }
        .ok_or_else(|| "AVFoundation audio capture is unavailable.".to_string())?;
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio_media_type) };
    if status == AVAuthorizationStatus::Authorized {
        return Ok(true);
    }
    if status == AVAuthorizationStatus::Denied || status == AVAuthorizationStatus::Restricted {
        return Ok(false);
    }

    let (sender, receiver) = mpsc::channel();
    let completion = RcBlock::new(move |granted: objc2::runtime::Bool| {
        let _ = sender.send(granted.as_bool());
    });
    unsafe {
        AVCaptureDevice::requestAccessForMediaType_completionHandler(audio_media_type, &completion);
    }
    receiver
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| "Microphone permission request timed out.".to_string())
}

#[cfg(not(target_os = "macos"))]
#[tauri::command]
pub fn request_microphone_access() -> Result<bool, String> {
    Ok(true)
}

impl CodexAppServerClient {
    fn spawn(
        app: AppHandle,
        thread_id: String,
        runtime_env: &CodexVoiceRuntimeEnv,
    ) -> Result<Self, String> {
        let binary = resolve_command("codex")
            .ok_or_else(|| "The Codex runtime could not be found.".to_string())?;
        let mut command = Command::new(binary);
        configure_voice_runtime_env(&mut command, runtime_env);
        let mut child = command
            .args(["app-server", "--stdio", "--enable", "realtime_conversation"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start the Codex voice runtime: {error}"))?;
        let stdin =
            Arc::new(Mutex::new(child.stdin.take().ok_or_else(|| {
                "Codex voice stdin is unavailable.".to_string()
            })?));
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex voice stdout is unavailable.".to_string())?;
        let stderr = child.stderr.take();
        let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Result<Value, String>>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);
        let expected_shutdown = Arc::new(AtomicBool::new(false));
        let reader_expected_shutdown = Arc::clone(&expected_shutdown);
        let reader_app = app.clone();
        let reader_thread_id = thread_id;

        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if let Some(id) = message.get("id").and_then(Value::as_u64) {
                    if let Ok(mut waiters) = reader_pending.lock() {
                        if let Some(sender) = waiters.remove(&id) {
                            let result = if let Some(error) = message.get("error") {
                                Err(json_rpc_error(error))
                            } else {
                                Ok(message.get("result").cloned().unwrap_or(Value::Null))
                            };
                            let _ = sender.send(result);
                        }
                    }
                    continue;
                }
                let Some(method) = message.get("method").and_then(Value::as_str) else {
                    continue;
                };
                if method.starts_with("thread/realtime/") {
                    let mut params = message.get("params").cloned().unwrap_or(Value::Null);
                    if let Some(params) = params.as_object_mut() {
                        params.insert(
                            "threadId".to_string(),
                            Value::String(reader_thread_id.clone()),
                        );
                    }
                    let _ = reader_app.emit(
                        VOICE_EVENT,
                        CodexVoiceEvent {
                            method: method.to_string(),
                            params,
                        },
                    );
                }
            }
            if !reader_expected_shutdown.load(Ordering::Acquire) {
                let _ = reader_app.emit(
                    VOICE_EVENT,
                    CodexVoiceEvent {
                        method: "thread/realtime/error".to_string(),
                        params: json!({
                            "threadId": reader_thread_id,
                            "message": "The Codex voice runtime closed unexpectedly."
                        }),
                    },
                );
            }
        });

        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    eprintln!("buzz-codex-voice: {line}");
                }
            });
        }

        Ok(Self {
            child,
            stdin,
            next_id: AtomicU64::new(1),
            pending,
            expected_shutdown,
        })
    }

    fn write(&self, message: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.lock().map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut *stdin, message)
            .map_err(|error| format!("Could not encode a Codex voice request: {error}"))?;
        stdin
            .write_all(b"\n")
            .and_then(|_| stdin.flush())
            .map_err(|error| format!("Could not send a Codex voice request: {error}"))
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({ "method": method, "params": params }))
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        self.request_with_timeout(method, params, REQUEST_TIMEOUT)
    }

    fn request_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        self.pending
            .lock()
            .map_err(|error| error.to_string())?
            .insert(id, sender);
        if let Err(error) = self.write(&json!({
            "id": id,
            "method": method,
            "params": params
        })) {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            return Err(error);
        }
        receiver.recv_timeout(timeout).map_err(|_| {
            if let Ok(mut pending) = self.pending.lock() {
                pending.remove(&id);
            }
            format!("Codex voice timed out while calling {method}.")
        })?
    }

    fn stop(mut self, thread_id: &str) {
        self.expected_shutdown.store(true, Ordering::Release);
        let _ = self.request_with_timeout(
            "thread/realtime/stop",
            json!({ "threadId": thread_id }),
            STOP_TIMEOUT,
        );
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for CodexAppServerClient {
    fn drop(&mut self) {
        self.expected_shutdown.store(true, Ordering::Release);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn json_rpc_error(error: &Value) -> String {
    error
        .get("message")
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .unwrap_or_else(|| error.to_string())
}

#[tauri::command]
pub fn get_codex_voice_status(app: AppHandle) -> Result<CodexVoiceStatus, String> {
    let state = app.state::<crate::app_state::AppState>();
    let guard = state
        .codex_voice
        .lock()
        .map_err(|error| error.to_string())?;
    let sessions = guard
        .sessions
        .values()
        .map(|session| CodexVoiceSessionStatus {
            thread_id: session.thread_id.clone(),
            muted: session.muted,
            model: REALTIME_MODEL.to_string(),
            mode: session.mode,
            voice: session.voice.clone(),
        })
        .collect::<Vec<_>>();
    Ok(CodexVoiceStatus {
        active: !sessions.is_empty(),
        muted: !sessions.is_empty() && sessions.iter().all(|session| session.muted),
        model: (!sessions.is_empty()).then(|| REALTIME_MODEL.to_string()),
        sessions,
    })
}

#[tauri::command]
pub fn start_codex_voice(
    thread_id: String,
    pubkey: String,
    agent_name: String,
    relay_url: String,
    voice: String,
    sdp: String,
    app: AppHandle,
) -> Result<CodexVoiceStartResponse, String> {
    if thread_id.is_empty() || sdp.is_empty() {
        return Err("The agent task and WebRTC offer are required.".to_string());
    }
    let runtime_env = codex_voice_runtime_env(&app, &pubkey, &relay_url)?
        .ok_or_else(|| "This agent is not using the native Codex runtime.".to_string())?;
    {
        let state = app.state::<crate::app_state::AppState>();
        if state
            .codex_voice
            .lock()
            .map_err(|error| error.to_string())?
            .sessions
            .contains_key(&thread_id)
        {
            return Err("Codex Voice is already active for this agent task.".to_string());
        }
    }

    let client = CodexAppServerClient::spawn(app.clone(), thread_id.clone(), &runtime_env)?;
    client.request(
        "initialize",
        json!({
            "clientInfo": {
                "name": "buzz_desktop",
                "title": "Buzz",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "experimentalApi": true,
                "requestAttestation": false
            }
        }),
    )?;
    client.notify("initialized", json!({}))?;
    let runtime_thread_id = match runtime_env.mode {
        CodexVoiceMode::Native => {
            client.request(
                "thread/resume",
                json!({
                    "threadId": thread_id,
                    "excludeTurns": true,
                    "approvalPolicy": "never"
                }),
            )?;
            thread_id.clone()
        }
        CodexVoiceMode::Proxy => client
            .request(
                "thread/start",
                json!({
                    "ephemeral": true,
                    "approvalPolicy": "never",
                    "developerInstructions": "This ephemeral thread is a speech transport owned by Buzz. Do not run tools or take independent action."
                }),
            )?
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .map(ToString::to_string)
            .ok_or_else(|| "Codex Voice could not create a proxy thread.".to_string())?,
    };
    let selected_voice = realtime_voice(&voice).to_string();
    let instructions = match runtime_env.mode {
        CodexVoiceMode::Native => format!(
            "You are speaking inside Buzz as {agent_name}. This realtime session is attached \
             directly to this managed agent's existing Codex task. Use that task's instructions, \
             workspace, tools, and current context. You are also in a shared voice room: you may \
             hear the user and other agents. Respond only when addressed or when you can add \
             material value; avoid acknowledgements and do not create conversational loops."
        ),
        CodexVoiceMode::Proxy => format!(
            "You are the realtime speech transport for the Buzz agent {agent_name}. Transcribe \
             every incoming user or peer utterance accurately. Never answer, acknowledge, \
             summarize, or speak on your own. The Buzz host supplies that agent's exact replies \
             through speech append operations. Remain silent after every incoming utterance."
        ),
    };
    client.request(
        "thread/realtime/start",
        json!({
            "threadId": runtime_thread_id,
            "model": REALTIME_MODEL,
            "version": "v3",
            "voice": selected_voice,
            "outputModality": "audio",
            "includeStartupContext": runtime_env.mode == CodexVoiceMode::Native,
            "flushTranscriptTailOnSessionEnd": true,
            "codexResponsesAsItems": runtime_env.mode == CodexVoiceMode::Native,
            "codexResponseHandoffMode": "thinking",
            "initialItems": [{
                "role": "developer",
                "text": instructions
            }],
            "transport": {
                "type": "webrtc",
                "sdp": sdp
            }
        }),
    )?;

    app.state::<crate::app_state::AppState>()
        .codex_voice
        .lock()
        .map_err(|error| error.to_string())?
        .sessions
        .insert(
            thread_id.clone(),
            CodexVoiceSession {
                thread_id,
                runtime_thread_id,
                muted: false,
                mode: runtime_env.mode,
                voice: selected_voice.clone(),
                client,
            },
        );

    Ok(CodexVoiceStartResponse {
        muted: false,
        model: REALTIME_MODEL.to_string(),
        mode: runtime_env.mode,
        voice: selected_voice,
    })
}

#[tauri::command]
pub fn speak_codex_voice(thread_id: String, text: String, app: AppHandle) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let state = app.state::<crate::app_state::AppState>();
    let guard = state
        .codex_voice
        .lock()
        .map_err(|error| error.to_string())?;
    let session = guard
        .sessions
        .get(&thread_id)
        .ok_or_else(|| "No Codex Voice session is active for this agent task.".to_string())?;
    session.client.request(
        "thread/realtime/appendSpeech",
        json!({
            "threadId": session.runtime_thread_id,
            "text": text
        }),
    )?;
    Ok(())
}

#[tauri::command]
pub fn set_codex_voice_muted(
    thread_id: String,
    muted: bool,
    app: AppHandle,
) -> Result<bool, String> {
    let state = app.state::<crate::app_state::AppState>();
    let mut guard = state
        .codex_voice
        .lock()
        .map_err(|error| error.to_string())?;
    let session = guard
        .sessions
        .get_mut(&thread_id)
        .ok_or_else(|| "No Codex Voice session is active for this agent task.".to_string())?;
    session.muted = muted;
    Ok(muted)
}

#[tauri::command]
pub fn stop_codex_voice(thread_id: String, app: AppHandle) -> Result<(), String> {
    let session = app
        .state::<crate::app_state::AppState>()
        .codex_voice
        .lock()
        .map_err(|error| error.to_string())?
        .sessions
        .remove(&thread_id);
    if let Some(session) = session {
        session.client.stop(&session.runtime_thread_id);
    }
    Ok(())
}

pub fn shutdown_codex_voice(app: &AppHandle) {
    let sessions = app
        .state::<crate::app_state::AppState>()
        .codex_voice
        .lock()
        .ok()
        .map(|mut state| std::mem::take(&mut state.sessions))
        .unwrap_or_default();
    for session in sessions.into_values() {
        session.client.stop(&session.runtime_thread_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        configure_voice_runtime_env, realtime_voice, resolve_voice_mode, resolve_voice_relay_url,
        CodexVoiceMode, CodexVoiceRuntimeEnv,
    };
    use std::{collections::HashMap, ffi::OsString, process::Command};

    fn configured_env(runtime_env: &CodexVoiceRuntimeEnv) -> HashMap<OsString, Option<OsString>> {
        let mut command = Command::new("codex");
        configure_voice_runtime_env(&mut command, runtime_env);
        command
            .get_envs()
            .map(|(key, value)| (key.to_os_string(), value.map(OsString::from)))
            .collect()
    }

    #[test]
    fn voice_runtime_receives_managed_agent_buzz_identity() {
        let env = configured_env(&CodexVoiceRuntimeEnv {
            private_key_nsec: "nsec-test".to_string(),
            auth_tag: Some("owner-attestation".to_string()),
            relay_url: "wss://relay.example".to_string(),
            mode: CodexVoiceMode::Native,
        });

        assert_eq!(
            env.get(&OsString::from("BUZZ_PRIVATE_KEY")),
            Some(&Some(OsString::from("nsec-test")))
        );
        assert_eq!(
            env.get(&OsString::from("NOSTR_PRIVATE_KEY")),
            Some(&Some(OsString::from("nsec-test")))
        );
        assert_eq!(
            env.get(&OsString::from("BUZZ_RELAY_URL")),
            Some(&Some(OsString::from("wss://relay.example")))
        );
        assert_eq!(
            env.get(&OsString::from("BUZZ_AUTH_TAG")),
            Some(&Some(OsString::from("owner-attestation")))
        );
    }

    #[test]
    fn voice_runtime_removes_inherited_auth_tag_for_legacy_agent() {
        let env = configured_env(&CodexVoiceRuntimeEnv {
            private_key_nsec: "nsec-test".to_string(),
            auth_tag: None,
            relay_url: "wss://relay.example".to_string(),
            mode: CodexVoiceMode::Proxy,
        });

        assert_eq!(env.get(&OsString::from("BUZZ_AUTH_TAG")), Some(&None));
    }

    #[test]
    fn voice_runtime_uses_active_workspace_relay_for_unpinned_agent() {
        assert_eq!(
            resolve_voice_relay_url("", "", "wss://workspace.example"),
            "wss://workspace.example"
        );
    }

    #[test]
    fn unsupported_realtime_voice_falls_back_to_sol() {
        assert_eq!(realtime_voice("cove"), "cove");
        assert_eq!(realtime_voice("cedar"), "sol");
        assert_eq!(realtime_voice("not-a-voice"), "sol");
    }

    #[test]
    fn voice_uses_proxy_transport_for_every_agent() {
        assert_eq!(resolve_voice_mode("Orchestrator", true), CodexVoiceMode::Proxy);
        assert_eq!(resolve_voice_mode("Builder", true), CodexVoiceMode::Proxy);
        assert_eq!(resolve_voice_mode("Explorer", false), CodexVoiceMode::Proxy);
    }
}
