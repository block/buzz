//! Loopback-only WebSocket listener. Frames (JSON text):
//!   server -> {"t":"challenge", ...}                     right after the upgrade
//!   client -> {"t":"auth","event":"<signed kind-22242 json>"}   purpose "connect"
//!   server -> {"t":"ready","desktopId":..,"terminals":bool} | {"t":"error","code":..} then close
//!   client -> {"t":"ping"} -> {"t":"pong"}                      (also keeps the idle timer alive)
//!   client -> {"t":"term.challenge","launchId":..}              -> {"t":"challenge", ...}
//!   client -> {"t":"term.open","launchId","cols","rows","session"?,"event"?:..} purpose "terminal:<launchId>"
//!     The first launch needs a signed `terminal:<launchId>` challenge; that starts a signed terminal window of the desktop's
//!     terminal lifetime, during which further launches on the same connection may omit `event`. After it ends, a launch
//!     without a fresh signature is refused with "term_signature_required" (the web then asks the signer again).
//!   server -> {"t":"term.opened","id"} / {"t":"term.out","id","data":b64} / {"t":"term.closed","id"}
//!   client -> {"t":"term.in","id","data":b64} / {"t":"term.resize","id","cols","rows"} / {"t":"term.close","id"}
//!   client -> {"t":"mgmt","id","op","params"}                   -> {"t":"mgmt.result","id","ok":bool,"result"|"error"}
//!     (agent management; refused with "agents_disabled" unless the desktop's "Allow remote agent management" is on)
//!   server -> {"t":"error","code":"session_expired"} then close  when the signed connection's lifetime ends
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use nostr::PublicKey;
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use crate::emu::{clamp_dims, EmuSession};
use crate::challenge::{now_secs, random_hex, ChallengeBook, Purpose};

const MAX_FRAME_BYTES: usize = 64 * 1024;
const MAX_TERM_INPUT: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct TerminalLaunch {
    pub launch_id: String,
    pub cols: u16,
    pub rows: u16,
    /// herdr session name to attach/launch with; None = the user's plain shell.
    pub session: Option<String>,
    /// What the web asked for on THIS launch: "shell" or "herdr". The desktop's own Buzz Term setting decides what
    /// is allowed (a request can always ask for less, never for herdr when the desktop doesn't use it).
    pub host: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub name: String,
    pub status: String,
}

pub enum TermInput {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
}

pub struct TerminalHandle {
    pub output: mpsc::Receiver<Vec<u8>>,
    pub input: mpsc::Sender<TermInput>,
    /// Lines of history the web should keep for this terminal (the desktop's Buzz Term scrollback setting).
    pub scrollback: Option<u32>,
}

/// Agent management requests (list/start/stop/message/create/rename/update/providers). Implemented by the app, which
/// owns the real agent store; a fake in tests. The server only checks auth, the switch, op shape and size.
pub trait ManagementBackend: Send + Sync {
    fn call(&self, op: String, params: Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>>;
}

/// Longest a signed connection may live (matches the desktop's "Signature lifetime" maximum, 24 h).
pub const MAX_SESSION_TTL: Duration = Duration::from_secs(24 * 3600);
const MGMT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_MGMT_INFLIGHT: usize = 4;
const MAX_MGMT_OP: usize = 48;

fn clamp_ttl(t: Duration) -> Duration {
    t.clamp(Duration::from_secs(1), MAX_SESSION_TTL)
}

/// Supplied by the app (real PTY/herdr launch); a fake in tests.
pub trait TerminalBackend: Send + Sync {
    fn open(&self, launch: TerminalLaunch) -> Result<TerminalHandle, String>;
    /// Named sessions the web may attach to (herdr). Empty for plain shells.
    fn list_sessions(&self) -> Vec<SessionInfo> {
        vec![]
    }
}

#[derive(Clone)]
pub struct ServerConfig {
    pub member: PublicKey,
    pub desktop_id: String,
    /// Exact `Origin` values allowed to upgrade (the PWA's origin). Browsers always send one.
    pub allowed_origins: Vec<String>,
    /// Non-browser clients (tests) have no Origin; keep false in the app.
    pub allow_missing_origin: bool,
    pub auth_timeout: Duration,
    pub idle_timeout: Duration,
    pub max_connections: usize,
    pub max_terminals: usize,
    pub backend: Option<Arc<dyn TerminalBackend>>,
    pub mgmt: Option<Arc<dyn ManagementBackend>>,
    /// How long one signed connection (one connect signature) stays valid; clamped to [`MAX_SESSION_TTL`].
    pub session_ttl: Duration,
    /// How long one terminal signature lets further launches skip the signer on that connection; clamped like `session_ttl`.
    pub term_session_ttl: Duration,
}

impl ServerConfig {
    pub fn new(member: PublicKey, desktop_id: impl Into<String>, allowed_origins: Vec<String>) -> Self {
        Self {
            member,
            desktop_id: desktop_id.into(),
            allowed_origins,
            allow_missing_origin: false,
            auth_timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(30 * 60),
            max_connections: 8,
            max_terminals: 3,
            backend: None,
            mgmt: None,
            session_ttl: Duration::from_secs(3600),
            term_session_ttl: Duration::from_secs(3600),
        }
    }
}

struct Shared {
    cfg: ServerConfig,
    book: Mutex<ChallengeBook>,
    terminals_enabled: AtomicBool,
    agents_enabled: AtomicBool,
    ttl_s: AtomicU64,
    term_ttl_s: AtomicU64,
    conns: AtomicUsize,
    terms: AtomicUsize,
    kill: broadcast::Sender<()>,
}

pub struct RemoteServer {
    pub addr: SocketAddr,
    shared: Arc<Shared>,
    stop: broadcast::Sender<()>,
}

impl RemoteServer {
    /// Binds 127.0.0.1 only (port 0 = ephemeral).
    pub async fn start(cfg: ServerConfig, port: u16) -> std::io::Result<RemoteServer> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port)).await?;
        let addr = listener.local_addr()?;
        debug_assert!(addr.ip().is_loopback());
        let (kill, _) = broadcast::channel(8);
        let (stop, _) = broadcast::channel(1);
        let ttl0 = clamp_ttl(cfg.session_ttl).as_secs();
        let term_ttl0 = clamp_ttl(cfg.term_session_ttl).as_secs();
        let shared = Arc::new(Shared {
            book: Mutex::new(ChallengeBook::new(cfg.member, cfg.desktop_id.clone())),
            cfg,
            terminals_enabled: AtomicBool::new(false), // terminals are off by default
            agents_enabled: AtomicBool::new(false),
            ttl_s: AtomicU64::new(ttl0),
            term_ttl_s: AtomicU64::new(term_ttl0),
            conns: AtomicUsize::new(0),
            terms: AtomicUsize::new(0),
            kill,
        });
        let sh = shared.clone();
        let mut stop_rx = stop.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop_rx.recv() => break,
                    r = listener.accept() => {
                        let Ok((sock, peer)) = r else { continue };
                        if !peer.ip().is_loopback() { continue; }
                        if sh.conns.load(Ordering::SeqCst) >= sh.cfg.max_connections { continue; }
                        sh.conns.fetch_add(1, Ordering::SeqCst);
                        let sh2 = sh.clone();
                        let mut stop2 = stop_rx.resubscribe();
                        tokio::spawn(async move {
                            tokio::select! {
                                _ = handle(sock, sh2.clone()) => {}
                                _ = stop2.recv() => {}
                            }
                            sh2.conns.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                }
            }
        });
        Ok(RemoteServer { addr, shared, stop })
    }

    /// The desktop's second, separate toggle. Turning it off also closes every open remote terminal.
    pub fn set_terminals_enabled(&self, on: bool) {
        self.shared.terminals_enabled.store(on, Ordering::SeqCst);
        if !on {
            let _ = self.shared.kill.send(());
        }
    }
    /// "Allow remote agent management". Off by default; checked on every request, so turning it off takes effect at once.
    pub fn set_agents_enabled(&self, on: bool) {
        self.shared.agents_enabled.store(on, Ordering::SeqCst);
    }
    pub fn agents_enabled(&self) -> bool {
        self.shared.agents_enabled.load(Ordering::SeqCst)
    }
    /// Changes the signature lifetime applied to connections authenticated from now on (clamped to 24 h).
    pub fn set_session_ttl(&self, ttl: Duration) {
        self.shared.ttl_s.store(clamp_ttl(ttl).as_secs(), Ordering::SeqCst);
    }
    /// Changes the terminal signature lifetime for terminal signatures made from now on (clamped to 24 h).
    pub fn set_terminal_session_ttl(&self, ttl: Duration) {
        self.shared.term_ttl_s.store(clamp_ttl(ttl).as_secs(), Ordering::SeqCst);
    }
    pub fn terminals_enabled(&self) -> bool {
        self.shared.terminals_enabled.load(Ordering::SeqCst)
    }
    pub fn open_terminals(&self) -> usize {
        self.shared.terms.load(Ordering::SeqCst)
    }
    pub fn connections(&self) -> usize {
        self.shared.conns.load(Ordering::SeqCst)
    }
    /// "Kill list": closes every remote terminal now.
    pub fn kill_all_terminals(&self) {
        let _ = self.shared.kill.send(());
    }
    pub fn shutdown(&self) {
        let _ = self.stop.send(());
    }
}

fn err(code: &str) -> Message {
    Message::text(json!({"t":"error","code":code}).to_string())
}

fn challenge_frame(sh: &Shared, conn_id: &str, purpose: Option<&str>) -> Message {
    let c = sh.book.lock().unwrap_or_else(|p| p.into_inner()).issue(conn_id, now_secs());
    let mut v = serde_json::to_value(&c).unwrap();
    v["t"] = "challenge".into();
    if let Some(p) = purpose {
        v["purpose"] = p.into();
    }
    Message::text(v.to_string())
}

async fn handle(sock: TcpStream, sh: Arc<Shared>) {
    let allowed = sh.cfg.allowed_origins.clone();
    let allow_missing = sh.cfg.allow_missing_origin;
    let cb = move |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok());
        let ok = match origin {
            Some(o) => allowed.iter().any(|a| a == o),
            None => allow_missing,
        };
        if ok {
            Ok(resp)
        } else {
            let mut r = ErrorResponse::new(Some("origin not allowed".into()));
            *r.status_mut() = StatusCode::FORBIDDEN;
            Err(r)
        }
    };
    let cfg = WebSocketConfig::default().max_message_size(Some(MAX_FRAME_BYTES)).max_frame_size(Some(MAX_FRAME_BYTES));
    let Ok(ws) = tokio_tungstenite::accept_hdr_async_with_config(sock, cb, Some(cfg)).await else { return };
    let (mut tx, mut rx) = ws.split();
    let conn_id = random_hex(8);
    let (out_tx, mut out_rx) = mpsc::channel::<Message>(64);
    let writer = tokio::spawn(async move {
        while let Some(m) = out_rx.recv().await {
            let close = matches!(m, Message::Close(_));
            if tx.send(m).await.is_err() || close {
                break;
            }
        }
    });
    let _ = out_tx.send(challenge_frame(&sh, &conn_id, Some("connect"))).await;

    // 1. authenticate (nothing but "auth" is accepted before this)
    let authed = tokio::time::timeout(sh.cfg.auth_timeout, async {
        while let Some(Ok(m)) = rx.next().await {
            let Message::Text(t) = m else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&t) else { return Err("bad_frame") };
            if v["t"] != "auth" {
                return Err("auth_required");
            }
            let ev = v["event"].as_str().unwrap_or("");
            return match sh.book.lock().unwrap_or_else(|p| p.into_inner()).verify(ev, &conn_id, now_secs()) {
                Ok(Purpose::Connect) => Ok(()),
                Ok(_) => Err("wrong_purpose"),
                Err(e) => Err(e.code()),
            };
        }
        Err("closed")
    })
    .await
    .unwrap_or(Err("auth_timeout"));
    if let Err(code) = authed {
        let _ = out_tx.send(err(code)).await;
        let _ = out_tx.send(Message::Close(None)).await;
        sh.book.lock().unwrap_or_else(|p| p.into_inner()).drop_connection(&conn_id);
        let _ = writer.await;
        return;
    }
    let ttl = Duration::from_secs(sh.ttl_s.load(Ordering::SeqCst));
    let _ = out_tx
        .send(Message::text(
            json!({
                "t":"ready",
                "desktopId":sh.cfg.desktop_id,
                "terminals":sh.terminals_enabled.load(Ordering::SeqCst),
                "agents":sh.agents_enabled.load(Ordering::SeqCst),
                "sessionTtlS":ttl.as_secs(),
                "signedUntil":now_secs() + ttl.as_secs(),
                "termSessionTtlS":sh.term_ttl_s.load(Ordering::SeqCst),
            })
            .to_string(),
        ))
        .await;
    let expires = tokio::time::Instant::now() + ttl;
    let mgmt_inflight = Arc::new(AtomicUsize::new(0));
    // End of this connection's signed terminal window (unix seconds); None until a terminal launch is signed.
    let mut term_signed_until: Option<u64> = None;

    // 2. authenticated loop
    let mut terms: HashMap<String, mpsc::Sender<TermInput>> = HashMap::new();
    let mut emus: HashMap<String, Arc<EmuSession>> = HashMap::new();
    let mut kill = sh.kill.subscribe();
    let b64 = base64::engine::general_purpose::STANDARD;
    loop {
        let next = tokio::select! {
            m = tokio::time::timeout(sh.cfg.idle_timeout, rx.next()) => m,
            _ = tokio::time::sleep_until(expires) => {
                // the signed connection's lifetime is over: the web has to sign a new connect challenge
                let _ = out_tx.send(err("session_expired")).await;
                break;
            }
            _ = kill.recv() => {
                for (id, t) in terms.drain() { if let Some(e) = emus.remove(&id) { e.shutdown(); } let _ = t.send(TermInput::Close).await; let _ = out_tx.send(Message::text(json!({"t":"term.closed","id":id}).to_string())).await; }
                continue;
            }
        };
        let Ok(Some(Ok(m))) = next else {
            let _ = out_tx.send(err("idle_timeout")).await;
            break;
        };
        let Message::Text(t) = m else {
            if matches!(m, Message::Close(_)) {
                break;
            }
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&t) else { continue };
        match v["t"].as_str().unwrap_or("") {
            "ping" => {
                let _ = out_tx.send(Message::text(json!({"t":"pong"}).to_string())).await;
            }
            "sessions.list" => {
                // read-only metadata, but still only for authenticated connections with terminals enabled
                if !sh.terminals_enabled.load(Ordering::SeqCst) {
                    let _ = out_tx.send(err("terminals_disabled")).await;
                    continue;
                }
                let list = match sh.cfg.backend.clone() {
                    Some(b) => tokio::task::spawn_blocking(move || b.list_sessions()).await.unwrap_or_default(),
                    None => vec![],
                };
                let _ = out_tx.send(Message::text(json!({"t":"sessions","sessions":list}).to_string())).await;
            }
            "mgmt" => {
                let id = v["id"].as_str().unwrap_or("").to_string();
                let fail = |code: &str| Message::text(json!({"t":"mgmt.result","id":id,"ok":false,"error":code}).to_string());
                let op = v["op"].as_str().unwrap_or("").to_string();
                if id.is_empty() || id.len() > 64 || op.is_empty() || op.len() > MAX_MGMT_OP || !op.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_') {
                    let _ = out_tx.send(fail("bad_request")).await;
                    continue;
                }
                if !sh.agents_enabled.load(Ordering::SeqCst) {
                    let _ = out_tx.send(fail("agents_disabled")).await;
                    continue;
                }
                let Some(backend) = sh.cfg.mgmt.clone() else {
                    let _ = out_tx.send(fail("no_backend")).await;
                    continue;
                };
                if mgmt_inflight.fetch_add(1, Ordering::SeqCst) >= MAX_MGMT_INFLIGHT {
                    mgmt_inflight.fetch_sub(1, Ordering::SeqCst);
                    let _ = out_tx.send(fail("busy")).await;
                    continue;
                }
                let params = v.get("params").cloned().unwrap_or(Value::Null);
                let (out2, inflight, sh2) = (out_tx.clone(), mgmt_inflight.clone(), sh.clone());
                tokio::spawn(async move {
                    let r = tokio::time::timeout(MGMT_TIMEOUT, backend.call(op, params)).await;
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    // re-check: the switch may have been turned off while the request ran; the result is then withheld
                    let m = if !sh2.agents_enabled.load(Ordering::SeqCst) {
                        json!({"t":"mgmt.result","id":id,"ok":false,"error":"agents_disabled"})
                    } else {
                        match r {
                            Ok(Ok(res)) => json!({"t":"mgmt.result","id":id,"ok":true,"result":res}),
                            Ok(Err(e)) => json!({"t":"mgmt.result","id":id,"ok":false,"error":e.chars().take(300).collect::<String>()}),
                            Err(_) => json!({"t":"mgmt.result","id":id,"ok":false,"error":"timeout"}),
                        }
                    };
                    let _ = out2.send(Message::text(m.to_string())).await;
                });
            }
            "term.challenge" => {
                if !sh.terminals_enabled.load(Ordering::SeqCst) {
                    let _ = out_tx.send(err("terminals_disabled")).await;
                    continue;
                }
                let lid = v["launchId"].as_str().unwrap_or("");
                match Purpose::parse(&format!("terminal:{lid}")) {
                    Some(p) => {
                        let _ = out_tx.send(challenge_frame(&sh, &conn_id, Some(&p.as_tag()))).await;
                    }
                    None => {
                        let _ = out_tx.send(err("bad_launch_id")).await;
                    }
                }
            }
            "term.open" => {
                let lid = v["launchId"].as_str().unwrap_or("").to_string();
                let reply = open_terminal(&sh, &conn_id, &v, &lid, &mut terms, &mut emus, &out_tx, &mut term_signed_until).await;
                if let Err(code) = reply {
                    let _ = out_tx.send(Message::text(json!({"t":"error","code":code,"id":lid}).to_string())).await;
                }
            }
            "term.in" => {
                if let (Some(id), Some(d)) = (v["id"].as_str(), v["data"].as_str()) {
                    if let (Some(t), Ok(bytes)) = (terms.get(id), b64.decode(d)) {
                        if bytes.len() <= MAX_TERM_INPUT {
                            let _ = t.send(TermInput::Data(bytes)).await;
                        }
                    }
                }
            }
            "term.resize" => {
                if let Some(t) = v["id"].as_str().and_then(|id| terms.get(id)) {
                    let (c, r) = (v["cols"].as_u64().unwrap_or(80), v["rows"].as_u64().unwrap_or(24));
                    let _ = t.send(TermInput::Resize(c.clamp(10, 500) as u16, r.clamp(2, 200) as u16)).await;
                }
            }
            op if op.starts_with("emu.") => {
                let Some(id) = v["id"].as_str().map(str::to_string) else { continue };
                let Some(e) = emus.get(&id).cloned() else {
                    let _ = out_tx.send(emu_reply(&id, &v, Err("terminal session not found".into()))).await;
                    continue;
                };
                let rid_wanted = v.get("rid").is_some();
                let res: Result<Value, String> = match op {
                    "emu.input" => e.handle_input(v["data"].as_str().unwrap_or("")).await.map(|_| Value::Null),
                    "emu.resize" => {
                        let (c, r) = clamp_dims(v["cols"].as_u64().unwrap_or(80), v["rows"].as_u64().unwrap_or(24));
                        e.resize(c, r).await.map(|vp| serde_json::to_value(vp).unwrap_or(Value::Null))
                    }
                    "emu.viewport_ready" => match serde_json::from_value(v["viewport"].clone()) {
                        Ok(vp) => e.viewport_ready(v["sub"].as_str().unwrap_or(""), vp).map(|_| Value::Null),
                        Err(_) => Err("bad viewport".into()),
                    },
                    "emu.ack" => e.ack(v["sub"].as_str().unwrap_or(""), v["seq"].as_u64().unwrap_or(0)).map(|_| Value::Null),
                    "emu.scroll" => e.scroll(v["lines"].as_i64().unwrap_or(0).clamp(-10_000, 10_000) as i32).await.map(|_| Value::Null),
                    "emu.mouse" => e.mouse(&v["req"]).await.map(Value::Bool),
                    "emu.focus" => e.focus(v["focused"].as_bool().unwrap_or(false)).await.map(|_| Value::Null),
                    "emu.attach" => e.attach().map(|(sub, vp)| json!({"subscriptionId":sub.to_string(),"viewport":vp})),
                    "emu.detach" => e.detach(v["sub"].as_str().unwrap_or("")).map(|_| Value::Null),
                    "emu.close" => {
                        if let Some(e) = emus.remove(&id) {
                            e.shutdown();
                        }
                        if let Some(t) = terms.remove(&id) {
                            let _ = t.send(TermInput::Close).await;
                        }
                        Ok(Value::Null)
                    }
                    _ => Err("unknown op".into()),
                };
                if rid_wanted || res.is_err() {
                    let _ = out_tx.send(emu_reply(&id, &v, res)).await;
                }
            }
            "term.close" => {
                if let Some(id) = v["id"].as_str() {
                    if let Some(e) = emus.remove(id) {
                        e.shutdown();
                    }
                    if let Some(t) = terms.remove(id) {
                        let _ = t.send(TermInput::Close).await;
                    }
                }
            }
            _ => {}
        }
    }
    for (id, t) in terms.drain() {
        if let Some(e) = emus.remove(&id) {
            e.shutdown();
        }
        let _ = t.send(TermInput::Close).await;
    }
    sh.book.lock().unwrap_or_else(|p| p.into_inner()).drop_connection(&conn_id);
    let _ = out_tx.send(Message::Close(None)).await;
    let _ = writer.await;
}

async fn open_terminal(
    sh: &Arc<Shared>,
    conn_id: &str,
    v: &Value,
    lid: &str,
    terms: &mut HashMap<String, mpsc::Sender<TermInput>>,
    emus: &mut HashMap<String, Arc<EmuSession>>,
    out_tx: &mpsc::Sender<Message>,
    term_signed_until: &mut Option<u64>,
) -> Result<(), &'static str> {
    if !sh.terminals_enabled.load(Ordering::SeqCst) {
        return Err("terminals_disabled");
    }
    let backend = sh.cfg.backend.clone().ok_or("no_backend")?;
    let want = Purpose::parse(&format!("terminal:{lid}")).ok_or("bad_launch_id")?;
    let event = v["event"].as_str().unwrap_or("");
    let now = now_secs();
    if event.is_empty() {
        // no signature: only allowed inside this connection's signed terminal window
        if !term_window_open(*term_signed_until, now) {
            return Err("term_signature_required");
        }
    } else {
        // a signature for THIS launch; a connection signature is not enough. It (re)starts the signed terminal window.
        let got = sh
            .book
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .verify(event, conn_id, now)
            .map_err(|e| e.code())?;
        if got != want {
            return Err("wrong_purpose");
        }
        *term_signed_until = Some(now + sh.term_ttl_s.load(Ordering::SeqCst));
    }
    let signed_until = term_signed_until.unwrap_or(0);
    if terms.contains_key(lid) {
        return Err("already_open");
    }
    // atomically reserve a slot
    if sh.terms.fetch_add(1, Ordering::SeqCst) >= sh.cfg.max_terminals {
        sh.terms.fetch_sub(1, Ordering::SeqCst);
        return Err("too_many_terminals");
    }
    let session = v["session"].as_str().map(str::to_string).filter(|s| !s.is_empty());
    if let Some(s) = &session {
        if s.len() > 64 || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.') {
            sh.terms.fetch_sub(1, Ordering::SeqCst);
            return Err("bad_session");
        }
    }
    let launch_cols = v["cols"].as_u64().unwrap_or(80).clamp(10, 500) as u16;
    let launch_rows = v["rows"].as_u64().unwrap_or(24).clamp(2, 200) as u16;
    let launch = TerminalLaunch {
        launch_id: lid.to_string(),
        cols: v["cols"].as_u64().unwrap_or(80).clamp(10, 500) as u16,
        rows: v["rows"].as_u64().unwrap_or(24).clamp(2, 200) as u16,
        session,
        host: v["host"].as_str().filter(|h| matches!(*h, "shell" | "herdr")).map(str::to_string),
    };
    let handle = match backend.open(launch) {
        Ok(h) => h,
        Err(_) => {
            sh.terms.fetch_sub(1, Ordering::SeqCst);
            return Err("launch_failed");
        }
    };
    terms.insert(lid.to_string(), handle.input.clone());
    if v["emu"].as_bool().unwrap_or(false) {
        let scrollback = handle.scrollback;
        let (c, r) = (launch_cols, launch_rows);
        match EmuSession::start(lid, handle, c, r, out_tx.clone()) {
            Ok((session, sub, viewport, done)) => {
                emus.insert(lid.to_string(), session);
                let _ = out_tx
                    .send(Message::text(json!({"t":"term.opened","id":lid,"emu":true,"termSignedUntil":signed_until,"scrollback":scrollback,"subscriptionId":sub.to_string(),"viewport":viewport}).to_string()))
                    .await;
                let (sh2, out2, id) = (sh.clone(), out_tx.clone(), lid.to_string());
                tokio::spawn(async move {
                    let _ = done.await;
                    sh2.terms.fetch_sub(1, Ordering::SeqCst);
                    let _ = out2.send(Message::text(json!({"t":"term.closed","id":id}).to_string())).await;
                });
                return Ok(());
            }
            Err(_) => {
                terms.remove(lid);
                sh.terms.fetch_sub(1, Ordering::SeqCst);
                return Err("launch_failed");
            }
        }
    }
    let _ = out_tx.send(Message::text(json!({"t":"term.opened","id":lid,"termSignedUntil":signed_until,"scrollback":handle.scrollback}).to_string())).await;
    let (sh2, out2, id) = (sh.clone(), out_tx.clone(), lid.to_string());
    let mut output = handle.output;
    tokio::spawn(async move {
        let b64 = base64::engine::general_purpose::STANDARD;
        while let Some(chunk) = output.recv().await {
            let m = Message::text(json!({"t":"term.out","id":id,"data":b64.encode(chunk)}).to_string());
            if out2.send(m).await.is_err() {
                break;
            }
        }
        sh2.terms.fetch_sub(1, Ordering::SeqCst);
        let _ = out2.send(Message::text(json!({"t":"term.closed","id":id}).to_string())).await;
    });
    Ok(())
}

fn emu_reply(id: &str, v: &Value, res: Result<Value, String>) -> Message {
    let rid = v.get("rid").cloned().unwrap_or(Value::Null);
    Message::text(match res {
        Ok(r) => json!({"t":"emu.reply","id":id,"rid":rid,"ok":true,"result":r}),
        Err(e) => json!({"t":"emu.reply","id":id,"rid":rid,"ok":false,"error":e.chars().take(200).collect::<String>()}),
    }
    .to_string())
}

/// True while a connection's signed terminal window is open at `now` (unix seconds).
pub fn term_window_open(until: Option<u64>, now: u64) -> bool {
    until.is_some_and(|u| now < u)
}

#[cfg(test)]
mod window_tests {
    use super::term_window_open;
    #[test]
    fn terminal_window_needs_a_signature_and_expires() {
        assert!(!term_window_open(None, 100));
        assert!(term_window_open(Some(200), 100));
        assert!(term_window_open(Some(200), 199));
        assert!(!term_window_open(Some(200), 200));
        assert!(!term_window_open(Some(200), 500));
    }
}
