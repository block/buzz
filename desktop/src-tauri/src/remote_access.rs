//! PWA bridge slice B wiring (docs/pwa-bridge-slice-b-c-design.md): owns the loopback listener, the single
//! `tailscale serve` entry on https:9443, and the "Allow remote access" / "Allow remote terminals" switches.
//! The user manages Tailscale; this module only detects it and never installs, starts or signs in to it.
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use buzz_remote_access::server::{SessionInfo, TerminalHandle};
use buzz_remote_access::tailscale::{TickOutcome, SERVE_PORT};
use buzz_remote_access::{
    PtyBackend, RemoteServer, ServeManager, ServerConfig, SystemTailscale, TerminalBackend, TerminalLaunch,
    TerminalMode,
};
use nostr::PublicKey;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex as AsyncMutex;

use crate::app_state::AppState;

const POLL: Duration = Duration::from_secs(10);

pub struct RemoteAccessState {
    inner: Arc<AsyncMutex<Inner>>,
    herdr: Arc<AtomicBool>,
}

struct Inner {
    mgr: ServeManager<SystemTailscale>,
    server: Option<RemoteServer>,
    terminals: bool,
    poll: Option<tauri::async_runtime::JoinHandle<()>>,
    cleaned: bool,
    url: Option<String>,
}

impl Default for RemoteAccessState {
    fn default() -> Self {
        Self {
            inner: Arc::new(AsyncMutex::new(Inner {
                mgr: ServeManager::new(SystemTailscale),
                server: None,
                terminals: false,
                poll: None,
                cleaned: false,
                url: None,
            })),
            herdr: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessStatus {
    tailscale_up: bool,
    /// Why the switch is disabled (Tailscale missing / stopped / signed out).
    reason: Option<String>,
    dns_name: Option<String>,
    enabled: bool,
    terminals: bool,
    url: Option<String>,
    open_terminals: usize,
}

fn herdr_bin() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".local/bin/herdr"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("herdr"))
}

/// Mode is read per launch so changing the Buzz Term setting applies to the next terminal. The app's own
/// environment is never passed to the shell (see PtyBackend).
struct AppBackend {
    app: AppHandle,
    herdr: Arc<AtomicBool>,
}

impl AppBackend {
    fn pty(&self) -> PtyBackend {
        PtyBackend::new(if self.herdr.load(Ordering::SeqCst) {
            TerminalMode::Herdr { bin: herdr_bin() }
        } else {
            TerminalMode::Shell
        })
    }
}

impl TerminalBackend for AppBackend {
    fn open(&self, launch: TerminalLaunch) -> Result<TerminalHandle, String> {
        let session = launch.session.clone();
        let h = self.pty().open(launch)?;
        // Local notification: someone just opened a terminal on this Mac.
        let _ = self.app.emit("remote-terminal-opened", serde_json::json!({ "session": session }));
        Ok(h)
    }
    fn list_sessions(&self) -> Vec<SessionInfo> {
        if self.herdr.load(Ordering::SeqCst) {
            self.pty().list_sessions()
        } else {
            vec![]
        }
    }
}

fn allowed_origins(dns: Option<&str>) -> Vec<String> {
    let mut v = vec!["https://buzz.huladesk.com".to_string()];
    if let Some(d) = dns {
        // the Mac's own dev/preview serve entries (443 and 8443) on this tailnet name
        v.push(format!("https://{d}"));
        v.push(format!("https://{d}:8443"));
    }
    if let Ok(extra) = std::env::var("BUZZ_REMOTE_ORIGINS") {
        v.extend(extra.split(',').map(|s| s.trim().to_string()).filter(|s| s.starts_with("https://")));
    }
    v
}

async fn snapshot(inner: &Inner) -> RemoteAccessStatus {
    let ts = inner.mgr.status().await;
    RemoteAccessStatus {
        tailscale_up: ts.up,
        reason: ts.reason,
        dns_name: ts.dns_name,
        enabled: inner.mgr.enabled() && inner.server.is_some(),
        terminals: inner.terminals && inner.server.is_some(),
        url: inner.url.clone(),
        open_terminals: inner.server.as_ref().map_or(0, |s| s.open_terminals()),
    }
}

#[tauri::command]
pub async fn remote_access_status(state: State<'_, RemoteAccessState>) -> Result<RemoteAccessStatus, String> {
    let mut inner = state.inner.lock().await;
    if !inner.cleaned {
        inner.cleaned = true;
        if !inner.mgr.enabled() {
            inner.mgr.cleanup_stale().await; // a crash may have left our 9443 entry behind
        }
    }
    Ok(snapshot(&inner).await)
}

#[tauri::command]
pub async fn remote_access_set_enabled(
    app: AppHandle,
    state: State<'_, RemoteAccessState>,
    app_state: State<'_, AppState>,
    enabled: bool,
    desktop_id: String,
    herdr: bool,
) -> Result<RemoteAccessStatus, String> {
    state.herdr.store(herdr, Ordering::SeqCst);
    let mut inner = state.inner.lock().await;
    if !enabled {
        stop(&mut inner).await;
        return Ok(snapshot(&inner).await);
    }
    if inner.server.is_some() {
        return Ok(snapshot(&inner).await);
    }
    if desktop_id.is_empty() || desktop_id.len() > 64 {
        return Err("invalid desktop id".into());
    }
    let member: PublicKey = app_state.keys.lock().map_err(|e| e.to_string())?.public_key();
    let ts = inner.mgr.status().await;
    if !ts.up {
        return Err(ts.reason.unwrap_or_else(|| "Tailscale isn't running on this Mac.".into()));
    }
    let mut cfg = ServerConfig::new(member, desktop_id, allowed_origins(ts.dns_name.as_deref()));
    cfg.backend = Some(Arc::new(AppBackend { app: app.clone(), herdr: state.herdr.clone() }));
    let server = RemoteServer::start(cfg, 0).await.map_err(|e| e.to_string())?;
    let port = server.addr.port();
    if let Err(e) = inner.mgr.enable(port).await {
        server.shutdown();
        return Err(e);
    }
    inner.url = ts.dns_name.map(|d| format!("wss://{d}:{SERVE_PORT}/v1/ws"));
    inner.server = Some(server);
    inner.terminals = false; // terminals are a separate switch and start off every time
    let poll_inner = state.inner.clone();
    let poll_app = app.clone();
    inner.poll = Some(tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            let mut g = poll_inner.lock().await;
            if g.mgr.tick().await == TickOutcome::TurnedOff {
                // Tailscale dropped: serve entry already removed; stay off until the user re-enables.
                if let Some(s) = g.server.take() {
                    s.shutdown();
                }
                g.terminals = false;
                g.url = None;
                g.poll = None;
                let _ = poll_app.emit("remote-access-changed", ());
                break;
            }
        }
    }));
    Ok(snapshot(&inner).await)
}

async fn stop(inner: &mut Inner) {
    if let Some(p) = inner.poll.take() {
        p.abort();
    }
    if let Some(s) = inner.server.take() {
        s.set_terminals_enabled(false);
        s.shutdown();
    }
    inner.mgr.disable().await; // removes our serve entry on 9443 only
    inner.terminals = false;
    inner.url = None;
}

#[tauri::command]
pub async fn remote_access_set_terminals(
    state: State<'_, RemoteAccessState>,
    on: bool,
    herdr: bool,
) -> Result<RemoteAccessStatus, String> {
    state.herdr.store(herdr, Ordering::SeqCst);
    let mut inner = state.inner.lock().await;
    let Some(server) = inner.server.as_ref() else {
        return Err("Turn on Allow remote access first.".into());
    };
    server.set_terminals_enabled(on); // off also closes every open remote terminal
    inner.terminals = on;
    Ok(snapshot(&inner).await)
}

/// Quit: remove the serve entry and stop the listener.
pub fn shutdown_blocking(app: &AppHandle) {
    let inner = app.state::<RemoteAccessState>().inner.clone();
    tauri::async_runtime::block_on(async move {
        let _ = tokio::time::timeout(Duration::from_secs(8), async {
            let mut g = inner.lock().await;
            stop(&mut g).await;
        })
        .await;
    });
}
