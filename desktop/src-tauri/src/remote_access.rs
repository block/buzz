//! PWA bridge slice B wiring (docs/pwa-bridge-slice-b-c-design.md): owns the loopback listener, the single
//! `tailscale serve` entry on https:9443, and the "Allow remote access" / "Allow remote terminals" switches.
//! The user manages Tailscale; this module only detects it and never installs, starts or signs in to it.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use buzz_remote_access::server::{ManagementBackend, SessionInfo, TerminalHandle, MAX_SESSION_TTL};
use buzz_remote_access::tailscale::{TickOutcome, SERVE_PORT};
use buzz_remote_access::{
    PtyBackend, RemoteServer, ServeManager, ServerConfig, SystemTailscale, TerminalBackend, TerminalLaunch,
    TerminalMode,
};
use nostr::PublicKey;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{oneshot, Mutex as AsyncMutex};

use crate::app_state::AppState;

const POLL: Duration = Duration::from_secs(10);

/// Settings → Buzz Term as the desktop frontend last pushed them. Read at the moment each remote terminal launches
/// (there is no process-wide herdr flag any more); a launch may only ask for less than this allows.
#[derive(Clone, Debug, Default)]
pub struct TermConfig {
    /// Session host is herdr.
    pub herdr: bool,
    /// Blank = $SHELL.
    pub shell: String,
    /// Blank = home directory.
    pub cwd: String,
    pub scrollback: u32,
}

/// "Allow remote agent management" and the signature lifetime, as pushed by the frontend.
#[derive(Clone, Debug)]
struct AgentConfig {
    enabled: bool,
    ttl: Duration,
}

type Pending = Arc<StdMutex<HashMap<String, oneshot::Sender<Result<serde_json::Value, String>>>>>;

pub struct RemoteAccessState {
    inner: Arc<AsyncMutex<Inner>>,
    term: Arc<StdMutex<TermConfig>>,
    agents: Arc<StdMutex<AgentConfig>>,
    pending: Pending,
}

struct Inner {
    mgr: ServeManager<SystemTailscale>,
    server: Option<RemoteServer>,
    terminals: bool,
    agents: bool,
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
                agents: false,
                poll: None,
                cleaned: false,
                url: None,
            })),
            term: Arc::new(StdMutex::new(TermConfig::default())),
            agents: Arc::new(StdMutex::new(AgentConfig { enabled: false, ttl: Duration::from_secs(3600) })),
            pending: Arc::new(StdMutex::new(HashMap::new())),
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
    /// Direct agent management is on (the relay path is separate and always available when the desktop is online).
    agents: bool,
    url: Option<String>,
    open_terminals: usize,
}

fn herdr_bin() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".local/bin/herdr"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("herdr"))
}

/// Resolves one launch from the current Buzz Term settings plus what the web asked for. herdr only when the desktop's
/// own setting is herdr and the web didn't ask for a plain shell; the web can never turn herdr on by itself.
pub(crate) fn use_herdr(cfg: &TermConfig, requested: Option<&str>) -> bool {
    cfg.herdr && requested != Some("shell")
}

fn pty_for(cfg: &TermConfig, requested: Option<&str>) -> PtyBackend {
    let herdr = use_herdr(cfg, requested);
    let mut b = PtyBackend::new(if herdr { TerminalMode::Herdr { bin: herdr_bin() } } else { TerminalMode::Shell });
    let t = |s: &str| (!s.trim().is_empty()).then(|| PathBuf::from(s.trim()));
    b.shell = t(&cfg.shell);
    b.cwd = t(&cfg.cwd);
    // herdr keeps its own history; the setting applies to plain shells
    b.scrollback = (!herdr && cfg.scrollback > 0).then_some(cfg.scrollback);
    b
}

/// Settings are read per launch so changing Buzz Term applies to the next terminal. The app's own environment is
/// never passed to the shell (see PtyBackend).
struct AppBackend {
    app: AppHandle,
    term: Arc<StdMutex<TermConfig>>,
}

impl AppBackend {
    fn cfg(&self) -> TermConfig {
        self.term.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

impl TerminalBackend for AppBackend {
    fn open(&self, launch: TerminalLaunch) -> Result<TerminalHandle, String> {
        let session = launch.session.clone();
        let h = pty_for(&self.cfg(), launch.host.as_deref()).open(launch)?;
        // Local notification: someone just opened a terminal on this Mac.
        let _ = self.app.emit("remote-terminal-opened", serde_json::json!({ "session": session }));
        Ok(h)
    }
    fn list_sessions(&self) -> Vec<SessionInfo> {
        let cfg = self.cfg();
        if cfg.herdr {
            pty_for(&cfg, None).list_sessions()
        } else {
            vec![]
        }
    }
}

/// Agent management requests are executed by the desktop frontend (which owns the agent store and its validation):
/// each one is emitted as `remote-mgmt-request` and answered through `remote_mgmt_reply`.
struct AppMgmt {
    app: AppHandle,
    pending: Pending,
}

impl ManagementBackend for AppMgmt {
    fn call(&self, op: String, params: serde_json::Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send>> {
        let (app, pending) = (self.app.clone(), self.pending.clone());
        Box::pin(async move {
            let id = format!("{:016x}{:016x}", rand_u64(), rand_u64());
            let (tx, rx) = oneshot::channel();
            pending.lock().map_err(|e| e.to_string())?.insert(id.clone(), tx);
            if app.emit("remote-mgmt-request", serde_json::json!({ "reqId": id, "op": op, "params": params })).is_err() {
                if let Ok(mut g) = pending.lock() {
                    g.remove(&id);
                }
                return Err("desktop_unavailable".into());
            }
            let r = tokio::time::timeout(Duration::from_secs(100), rx).await;
            if let Ok(mut g) = pending.lock() {
                    g.remove(&id);
                }
            match r {
                Ok(Ok(v)) => v,
                Ok(Err(_)) => Err("desktop_unavailable".into()),
                Err(_) => Err("timeout".into()),
            }
        })
    }
}

fn rand_u64() -> u64 {
    let mut b = [0u8; 8];
    // request ids only need to be unguessable-ish and unique; fall back to the clock if the OS RNG fails
    if getrandom::getrandom(&mut b).is_err() {
        return std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    }
    u64::from_le_bytes(b)
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
        agents: inner.agents && inner.server.is_some(),
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
) -> Result<RemoteAccessStatus, String> {
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
    cfg.backend = Some(Arc::new(AppBackend { app: app.clone(), term: state.term.clone() }));
    cfg.mgmt = Some(Arc::new(AppMgmt { app: app.clone(), pending: state.pending.clone() }));
    let ac = state.agents.lock().map_err(|e| e.to_string())?.clone();
    cfg.session_ttl = ac.ttl;
    let server = RemoteServer::start(cfg, 0).await.map_err(|e| e.to_string())?;
    server.set_agents_enabled(ac.enabled);
    let port = server.addr.port();
    if let Err(e) = inner.mgr.enable(port).await {
        server.shutdown();
        return Err(e);
    }
    inner.url = ts.dns_name.map(|d| format!("wss://{d}:{SERVE_PORT}/v1/ws"));
    inner.server = Some(server);
    inner.terminals = false; // terminals are a separate switch and start off every time
    inner.agents = ac.enabled;
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
                g.agents = false;
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
    inner.agents = false;
    inner.url = None;
}

#[tauri::command]
pub async fn remote_access_set_terminals(
    state: State<'_, RemoteAccessState>,
    on: bool,
) -> Result<RemoteAccessStatus, String> {
    let mut inner = state.inner.lock().await;
    let Some(server) = inner.server.as_ref() else {
        return Err("Turn on Allow remote access first.".into());
    };
    server.set_terminals_enabled(on); // off also closes every open remote terminal
    inner.terminals = on;
    Ok(snapshot(&inner).await)
}

/// Pushes Settings → Buzz Term (session host, shell, default directory, scrollback). Applies to the NEXT remote
/// terminal; open ones are not touched.
#[tauri::command]
pub fn remote_access_set_term_config(
    state: State<'_, RemoteAccessState>,
    herdr: bool,
    shell: String,
    cwd: String,
    scrollback: u32,
) -> Result<(), String> {
    if shell.len() > 1024 || cwd.len() > 4096 {
        return Err("setting too long".into());
    }
    *state.term.lock().map_err(|e| e.to_string())? = TermConfig { herdr, shell, cwd, scrollback: scrollback.min(100_000) };
    Ok(())
}

/// "Allow remote agent management" + signature lifetime (seconds, clamped to 24 h). Takes effect on the running listener
/// at once for the switch (checked per request) and for connections authenticated from now on for the lifetime.
#[tauri::command]
pub async fn remote_access_set_agents(
    state: State<'_, RemoteAccessState>,
    on: bool,
    ttl_s: u64,
) -> Result<RemoteAccessStatus, String> {
    let ttl = Duration::from_secs(ttl_s).clamp(Duration::from_secs(60), MAX_SESSION_TTL);
    *state.agents.lock().map_err(|e| e.to_string())? = AgentConfig { enabled: on, ttl };
    let mut inner = state.inner.lock().await;
    if let Some(server) = inner.server.as_ref() {
        server.set_agents_enabled(on);
        server.set_session_ttl(ttl);
    }
    inner.agents = on && inner.server.is_some();
    Ok(snapshot(&inner).await)
}

/// The frontend's answer to a `remote-mgmt-request` event.
#[tauri::command]
pub fn remote_mgmt_reply(
    state: State<'_, RemoteAccessState>,
    req_id: String,
    ok: bool,
    result: Option<serde_json::Value>,
    error: Option<String>,
) -> Result<(), String> {
    let tx = state.pending.lock().map_err(|e| e.to_string())?.remove(&req_id);
    if let Some(tx) = tx {
        let _ = tx.send(if ok { Ok(result.unwrap_or(serde_json::Value::Null)) } else { Err(error.unwrap_or_else(|| "failed".into())) });
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn herdr_is_per_launch_and_never_requested_by_the_web_alone() {
        let herdr = TermConfig { herdr: true, ..Default::default() };
        let shell = TermConfig::default();
        assert!(use_herdr(&herdr, None));
        assert!(use_herdr(&herdr, Some("herdr")));
        assert!(!use_herdr(&herdr, Some("shell"))); // a launch may ask for less
        assert!(!use_herdr(&shell, Some("herdr"))); // never for more than the desktop allows
        assert!(!use_herdr(&shell, None));
    }

    #[test]
    fn buzz_term_settings_reach_the_pty_backend() {
        let cfg = TermConfig { herdr: false, shell: " /bin/zsh ".into(), cwd: "~/Projects".into(), scrollback: 2500 };
        let b = pty_for(&cfg, None);
        assert_eq!(b.shell, Some(PathBuf::from("/bin/zsh")));
        assert_eq!(b.cwd, Some(PathBuf::from("~/Projects")));
        assert_eq!(b.scrollback, Some(2500));
        let blank = pty_for(&TermConfig::default(), None);
        assert_eq!((blank.shell, blank.cwd, blank.scrollback), (None, None, None));
        // herdr launches keep herdr's own history
        assert_eq!(pty_for(&TermConfig { herdr: true, scrollback: 900, ..Default::default() }, None).scrollback, None);
    }
}
