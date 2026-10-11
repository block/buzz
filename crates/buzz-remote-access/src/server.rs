//! Loopback-only WebSocket listener. Frames (JSON text):
//!   server -> {"t":"challenge", ...}                     right after the upgrade
//!   client -> {"t":"auth","event":"<signed kind-22242 json>"}   purpose "connect"
//!   server -> {"t":"ready","desktopId":..,"terminals":bool} | {"t":"error","code":..} then close
//!   client -> {"t":"ping"} -> {"t":"pong"}                      (also keeps the idle timer alive)
//!   client -> {"t":"term.challenge","launchId":..}              -> {"t":"challenge", ...}
//!   client -> {"t":"term.open","launchId","cols","rows","session"?,"event":..} purpose "terminal:<launchId>"
//!   server -> {"t":"term.opened","id"} / {"t":"term.out","id","data":b64} / {"t":"term.closed","id"}
//!   client -> {"t":"term.in","id","data":b64} / {"t":"term.resize","id","cols","rows"} / {"t":"term.close","id"}
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
}

pub enum TermInput {
    Data(Vec<u8>),
    Resize(u16, u16),
    Close,
}

pub struct TerminalHandle {
    pub output: mpsc::Receiver<Vec<u8>>,
    pub input: mpsc::Sender<TermInput>,
}

/// Supplied by the app (real PTY/herdr launch); a fake in tests.
pub trait TerminalBackend: Send + Sync {
    fn open(&self, launch: TerminalLaunch) -> Result<TerminalHandle, String>;
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
        }
    }
}

struct Shared {
    cfg: ServerConfig,
    book: Mutex<ChallengeBook>,
    terminals_enabled: AtomicBool,
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
        let shared = Arc::new(Shared {
            book: Mutex::new(ChallengeBook::new(cfg.member, cfg.desktop_id.clone())),
            cfg,
            terminals_enabled: AtomicBool::new(false), // terminals are off by default
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
    let c = sh.book.lock().unwrap().issue(conn_id, now_secs());
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
            return match sh.book.lock().unwrap().verify(ev, &conn_id, now_secs()) {
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
        sh.book.lock().unwrap().drop_connection(&conn_id);
        let _ = writer.await;
        return;
    }
    let _ = out_tx
        .send(Message::text(json!({"t":"ready","desktopId":sh.cfg.desktop_id,"terminals":sh.terminals_enabled.load(Ordering::SeqCst)}).to_string()))
        .await;

    // 2. authenticated loop
    let mut terms: HashMap<String, mpsc::Sender<TermInput>> = HashMap::new();
    let mut kill = sh.kill.subscribe();
    let b64 = base64::engine::general_purpose::STANDARD;
    loop {
        let next = tokio::select! {
            m = tokio::time::timeout(sh.cfg.idle_timeout, rx.next()) => m,
            _ = kill.recv() => {
                for (id, t) in terms.drain() { let _ = t.send(TermInput::Close).await; let _ = out_tx.send(Message::text(json!({"t":"term.closed","id":id}).to_string())).await; }
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
                let reply = open_terminal(&sh, &conn_id, &v, &lid, &mut terms, &out_tx).await;
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
            "term.close" => {
                if let Some(id) = v["id"].as_str() {
                    if let Some(t) = terms.remove(id) {
                        let _ = t.send(TermInput::Close).await;
                    }
                }
            }
            _ => {}
        }
    }
    for (_, t) in terms.drain() {
        let _ = t.send(TermInput::Close).await;
    }
    sh.book.lock().unwrap().drop_connection(&conn_id);
    let _ = out_tx.send(Message::Close(None)).await;
    let _ = writer.await;
}

async fn open_terminal(
    sh: &Arc<Shared>,
    conn_id: &str,
    v: &Value,
    lid: &str,
    terms: &mut HashMap<String, mpsc::Sender<TermInput>>,
    out_tx: &mpsc::Sender<Message>,
) -> Result<(), &'static str> {
    if !sh.terminals_enabled.load(Ordering::SeqCst) {
        return Err("terminals_disabled");
    }
    let backend = sh.cfg.backend.clone().ok_or("no_backend")?;
    let want = Purpose::parse(&format!("terminal:{lid}")).ok_or("bad_launch_id")?;
    // a fresh signature for THIS launch; a connection signature is not enough
    let got = sh.book.lock().unwrap().verify(v["event"].as_str().unwrap_or(""), conn_id, now_secs()).map_err(|e| e.code())?;
    if got != want {
        return Err("wrong_purpose");
    }
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
    let launch = TerminalLaunch {
        launch_id: lid.to_string(),
        cols: v["cols"].as_u64().unwrap_or(80).clamp(10, 500) as u16,
        rows: v["rows"].as_u64().unwrap_or(24).clamp(2, 200) as u16,
        session,
    };
    let handle = match backend.open(launch) {
        Ok(h) => h,
        Err(_) => {
            sh.terms.fetch_sub(1, Ordering::SeqCst);
            return Err("launch_failed");
        }
    };
    terms.insert(lid.to_string(), handle.input.clone());
    let _ = out_tx.send(Message::text(json!({"t":"term.opened","id":lid}).to_string())).await;
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
