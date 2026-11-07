//! Chromium launcher and a minimal CDP client.
//!
//! The bridge does not vendor a browser-automation framework: it launches
//! the user's own Chromium with remote debugging enabled and speaks the
//! Chrome DevTools Protocol over a websocket. Only the CDP surface the
//! bridge needs is implemented: target management, navigation, and
//! `Runtime.evaluate` (all DOM interaction happens through evaluated
//! JavaScript from [`crate::selectors`]).
//!
//! CDP traffic is serialized through one mutex: request ids are matched
//! under the lock, so concurrent tabs cannot interleave a response into
//! another tab's call. That serializes turns across tabs at the protocol
//! layer; it is a deliberate v1 simplification, documented here so a
//! future per-connection design knows what it is replacing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_tungstenite::{
    connect_async, tungstenite::protocol::Message, MaybeTlsStream, WebSocketStream,
};

use crate::BridgeError;

/// Environment override for the Chromium binary path.
pub const CHROME_ENV: &str = "MUSE_ACP_CHROME";

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Connection {
    ws: Ws,
    next_id: u64,
}

impl Connection {
    /// Send one CDP command and await its matching response, ignoring
    /// interleaved events. `session_id` is `None` for browser-level
    /// commands (target management) and `Some` for page-level commands.
    async fn call(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, BridgeError> {
        let id = self.next_id;
        self.next_id += 1;
        let mut frame = json!({ "id": id, "method": method, "params": params });
        if let Some(session_id) = session_id {
            frame["sessionId"] = Value::String(session_id.to_string());
        }
        self.ws
            .send(Message::Text(frame.to_string().into()))
            .await
            .map_err(|e| BridgeError::Protocol(format!("cdp send failed: {e}")))?;

        let wait = async {
            loop {
                match self.ws.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = match serde_json::from_str(&text) {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        if value.get("id").and_then(|v| v.as_u64()) != Some(id) {
                            continue; // event or another call's response
                        }
                        if let Some(error) = value.get("error") {
                            let message = error
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("unknown CDP error");
                            return Err(BridgeError::Protocol(format!(
                                "{method} failed: {message}"
                            )));
                        }
                        return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = self.ws.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        return Err(BridgeError::Protocol(format!("cdp read failed: {e}")));
                    }
                    None => {
                        return Err(BridgeError::Protocol("browser connection closed".into()));
                    }
                }
            }
        };
        timeout(Duration::from_secs(60), wait)
            .await
            .map_err(|_| BridgeError::Protocol(format!("cdp call timed out: {method}")))?
    }
}

/// A launched Chromium instance. Owns the browser process: dropping the
/// browser kills it.
pub struct Browser {
    conn: std::sync::Arc<Mutex<Connection>>,
    child: Child,
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// One attached page target: a tab the bridge can navigate and evaluate in.
#[derive(Clone)]
pub struct Tab {
    conn: std::sync::Arc<Mutex<Connection>>,
    session_id: String,
    target_id: String,
}

impl Tab {
    /// Navigate the tab and wait for the page load event (best-effort: a
    /// timeout proceeds anyway, since single-page apps may never re-fire
    /// load after client-side routing).
    pub async fn navigate(&self, url: &str) -> Result<(), BridgeError> {
        let mut conn = self.conn.lock().await;
        conn.call("Page.enable", json!({}), Some(&self.session_id))
            .await?;
        conn.call(
            "Page.navigate",
            json!({ "url": url }),
            Some(&self.session_id),
        )
        .await?;
        let wait = async {
            loop {
                match conn.ws.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(value) = serde_json::from_str::<Value>(&text) else {
                            continue;
                        };
                        let is_load = value.get("method").and_then(|m| m.as_str())
                            == Some("Page.loadEventFired");
                        let is_ours = value.get("sessionId").and_then(|s| s.as_str())
                            == Some(self.session_id.as_str());
                        if is_load && is_ours {
                            return;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        let _ = conn.ws.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return,
                }
            }
        };
        // A missed load event must not strand the turn: proceed after 30s.
        let _ = timeout(Duration::from_secs(30), wait).await;
        Ok(())
    }

    /// Evaluate JavaScript in the tab and return the raw `result.value`.
    pub async fn evaluate(&self, expression: &str) -> Result<Value, BridgeError> {
        let mut conn = self.conn.lock().await;
        conn.call("Runtime.enable", json!({}), Some(&self.session_id))
            .await?;
        let result = conn
            .call(
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
                Some(&self.session_id),
            )
            .await?;
        if let Some(exception) = result.get("exceptionDetails") {
            let text = exception
                .get("exception")
                .and_then(|e| e.get("description"))
                .and_then(|d| d.as_str())
                .unwrap_or("javascript exception");
            return Err(BridgeError::Protocol(format!("evaluate failed: {text}")));
        }
        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(Value::Null))
    }

    /// Evaluate JavaScript that returns a JSON string, and parse it.
    pub async fn evaluate_json(&self, expression: &str) -> Result<Value, BridgeError> {
        let value = self.evaluate(expression).await?;
        let Some(text) = value.as_str() else {
            return Err(BridgeError::Protocol(
                "expected evaluated script to return a JSON string".into(),
            ));
        };
        serde_json::from_str(text)
            .map_err(|e| BridgeError::Protocol(format!("script returned invalid JSON: {e}")))
    }

    /// Close the tab's target.
    pub async fn close(&self) -> Result<(), BridgeError> {
        let mut conn = self.conn.lock().await;
        conn.call(
            "Target.closeTarget",
            json!({ "targetId": self.target_id }),
            None,
        )
        .await?;
        Ok(())
    }
}

/// Locate a Chromium binary: explicit env override, then `PATH`, then
/// well-known install locations.
pub fn find_chrome() -> Option<PathBuf> {
    if let Ok(path) = std::env::var(CHROME_ENV) {
        if !path.trim().is_empty() {
            return Some(PathBuf::from(path));
        }
    }
    let names: &[&str] = if cfg!(windows) {
        &["chrome.exe", "google-chrome.exe", "chromium.exe"]
    } else {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "chrome",
        ]
    };
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in names {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    let well_known: &[&str] = &[
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/snap/bin/chromium",
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
    ];
    well_known
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn free_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

/// Fetch `http://127.0.0.1:port/json/version` and return its
/// `webSocketDebuggerUrl`, polling until the debugger answers or the
/// deadline passes.
async fn wait_for_debugger_url(port: u16, child: &mut Child) -> Result<String, BridgeError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::time::Instant::now() > deadline {
            return Err(BridgeError::ChromeLaunch(
                "Chromium started but its debugging endpoint never answered".into(),
            ));
        }
        // If the browser exited, stop waiting and say so.
        if let Ok(Some(status)) = child.try_wait() {
            return Err(BridgeError::ChromeLaunch(format!(
                "Chromium exited during startup (status: {status})"
            )));
        }
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(mut stream) => {
                let request = b"GET /json/version HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n";
                if stream.write_all(request).await.is_err() {
                    continue;
                }
                let mut raw = Vec::new();
                if stream.read_to_end(&mut raw).await.is_err() {
                    continue;
                }
                let text = String::from_utf8_lossy(&raw);
                if let Some(body) = text.split("\r\n\r\n").nth(1) {
                    if let Ok(value) = serde_json::from_str::<Value>(body) {
                        if let Some(url) =
                            value.get("webSocketDebuggerUrl").and_then(|u| u.as_str())
                        {
                            return Ok(url.to_string());
                        }
                    }
                }
            }
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

/// Launch Chromium with a persistent profile and remote debugging, and
/// connect the CDP websocket.
pub async fn launch(headless: bool, profile_dir: &Path) -> Result<Browser, BridgeError> {
    let chrome = find_chrome().ok_or_else(|| {
        BridgeError::ChromeLaunch(
            "no Chromium found (checked MUSE_ACP_CHROME, PATH, and well-known install locations)"
                .into(),
        )
    })?;
    if !chrome.is_file() {
        return Err(BridgeError::ChromeLaunch(format!(
            "configured Chromium binary is not a file: {}",
            chrome.display()
        )));
    }
    let port = free_port()
        .map_err(|e| BridgeError::ChromeLaunch(format!("could not pick a debugging port: {e}")))?;
    let mut cmd = Command::new(&chrome);
    cmd.arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-gpu")
        .arg("--disable-dev-shm-usage")
        .arg("about:blank")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if headless {
        cmd.arg("--headless=new");
    }
    let mut child = cmd.spawn().map_err(|e| {
        BridgeError::ChromeLaunch(format!("failed to spawn {}: {e}", chrome.display()))
    })?;
    let ws_url = wait_for_debugger_url(port, &mut child).await?;
    let (ws, _) = connect_async(&ws_url)
        .await
        .map_err(|e| BridgeError::Protocol(format!("cdp websocket connect failed: {e}")))?;
    Ok(Browser {
        conn: std::sync::Arc::new(Mutex::new(Connection { ws, next_id: 1 })),
        child,
    })
}

impl Browser {
    /// Create a tab, attach to it, and navigate it to `url`.
    pub async fn new_tab(&self, url: &str) -> Result<Tab, BridgeError> {
        let mut conn = self.conn.lock().await;
        let created = conn
            .call("Target.createTarget", json!({ "url": "about:blank" }), None)
            .await?;
        let target_id = created
            .get("targetId")
            .and_then(|t| t.as_str())
            .ok_or_else(|| {
                BridgeError::Protocol("Target.createTarget returned no targetId".into())
            })?
            .to_string();
        let attached = conn
            .call(
                "Target.attachToTarget",
                json!({ "targetId": target_id, "flatten": true }),
                None,
            )
            .await?;
        let session_id = attached
            .get("sessionId")
            .and_then(|s| s.as_str())
            .ok_or_else(|| {
                BridgeError::Protocol("Target.attachToTarget returned no sessionId".into())
            })?
            .to_string();
        drop(conn);
        let tab = Tab {
            conn: self.conn.clone(),
            session_id,
            target_id,
        };
        tab.navigate(url).await?;
        Ok(tab)
    }
}
