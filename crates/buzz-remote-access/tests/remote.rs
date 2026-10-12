use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use buzz_remote_access::challenge::{now_secs, ChallengeBook, ChallengeError, Purpose};
use buzz_remote_access::server::{TermInput, TerminalHandle};
use buzz_remote_access::tailscale::{owns_entry, parse_status, ServeManager, TailscaleCli, TailscaleStatus, TickOutcome};
use buzz_remote_access::{RemoteServer, ServerConfig, TerminalBackend, TerminalLaunch};
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const ORIGIN: &str = "https://pwa.example";

fn sign(keys: &Keys, nonce: &str, conn: &str, desktop: &str, purpose: &str, ts: Option<u64>) -> String {
    let mut b = EventBuilder::new(Kind::Custom(22242), "").tags([
        Tag::parse(["challenge", nonce]).unwrap(),
        Tag::parse(["conn", conn]).unwrap(),
        Tag::parse(["desktop", desktop]).unwrap(),
        Tag::parse(["purpose", purpose]).unwrap(),
    ]);
    if let Some(t) = ts {
        b = b.custom_created_at(Timestamp::from_secs(t));
    }
    b.sign_with_keys(keys).unwrap().as_json()
}

#[test]
fn challenge_rules() {
    let me = Keys::generate();
    let other = Keys::generate();
    let mut book = ChallengeBook::new(me.public_key(), "dsk");
    let now = now_secs();
    let c = book.issue("c1", now);
    // wrong member
    assert_eq!(book.verify(&sign(&other, &c.nonce, "c1", "dsk", "connect", None), "c1", now), Err(ChallengeError::WrongMember));
    // the challenge survives a wrong-member attempt? it must not: verified only after sig/member, so still open
    let ok = sign(&me, &c.nonce, "c1", "dsk", "connect", None);
    assert_eq!(book.verify(&ok, "c1", now), Ok(Purpose::Connect));
    // single use
    assert_eq!(book.verify(&ok, "c1", now), Err(ChallengeError::UnknownChallenge));
    // other connection / desktop / purpose / expiry / stale
    let c = book.issue("c1", now);
    assert_eq!(book.verify(&sign(&me, &c.nonce, "c1", "dsk", "connect", None), "c2", now), Err(ChallengeError::WrongConnection));
    let c = book.issue("c1", now);
    assert_eq!(book.verify(&sign(&me, &c.nonce, "c1", "other-desktop", "connect", None), "c1", now), Err(ChallengeError::WrongDesktop));
    let c = book.issue("c1", now);
    assert_eq!(book.verify(&sign(&me, &c.nonce, "c1", "dsk", "shell", None), "c1", now), Err(ChallengeError::WrongPurpose));
    let c = book.issue("c1", now);
    assert_eq!(book.verify(&sign(&me, &c.nonce, "c1", "dsk", "connect", None), "c1", now + 61), Err(ChallengeError::Stale));
    let c = book.issue("c1", now);
    assert_eq!(book.verify(&sign(&me, &c.nonce, "c1", "dsk", "connect", Some(now + 100)), "c1", now + 100), Err(ChallengeError::Expired));
    assert_eq!(book.verify("{}", "c1", now), Err(ChallengeError::BadEvent));
    // tampered content/signature
    let c = book.issue("c1", now);
    let mut v: Value = serde_json::from_str(&sign(&me, &c.nonce, "c1", "dsk", "connect", None)).unwrap();
    v["tags"][3][1] = "terminal:abcdefgh".into();
    assert_eq!(book.verify(&v.to_string(), "c1", now), Err(ChallengeError::BadSignature));
    assert_eq!(Purpose::parse("terminal:../etc"), None);
    assert_eq!(Purpose::parse("terminal:abc"), None);
    assert_eq!(Purpose::parse("terminal:launch-0001"), Some(Purpose::Terminal("launch-0001".into())));
}

#[test]
fn tailscale_parsing() {
    let up = r#"{"BackendState":"Running","Self":{"DNSName":"mac.tailnet.ts.net.","TailscaleIPs":["100.1.2.3"]}}"#;
    let s = parse_status(up);
    assert!(s.up);
    assert_eq!(s.dns_name.as_deref(), Some("mac.tailnet.ts.net"));
    for (j, needle) in [
        (r#"{"BackendState":"Stopped"}"#, "turned off"),
        (r#"{"BackendState":"NeedsLogin"}"#, "sign in"),
        (r#"{"BackendState":"Running","Self":{"TailscaleIPs":[]}}"#, "no tailnet address"),
        ("garbage", "Couldn't read"),
    ] {
        let s = parse_status(j);
        assert!(!s.up && s.reason.unwrap().contains(needle));
    }
    let serve = r#"{"Web":{"mac.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:5199"}}},"mac.ts.net:9443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:41000"}}}}}"#;
    assert!(owns_entry(serve, 9443));
    assert!(!owns_entry(serve, 8443));
    let remote = r#"{"Web":{"mac.ts.net:9443":{"Handlers":{"/":{"Proxy":"https://example.com"}}}}}"#;
    assert!(!owns_entry(remote, 9443)); // not ours: points elsewhere
}

#[derive(Clone, Default)]
struct FakeTs {
    up: Arc<Mutex<bool>>,
    log: Arc<Mutex<Vec<String>>>,
    serve_json: Arc<Mutex<Option<String>>>,
}
impl TailscaleCli for FakeTs {
    async fn status(&self) -> TailscaleStatus {
        if *self.up.lock().unwrap() {
            TailscaleStatus { up: true, dns_name: Some("m.ts.net".into()), addrs: vec!["100.1.1.1".into()], reason: None }
        } else {
            TailscaleStatus::down("Tailscale is turned off on this Mac.")
        }
    }
    async fn serve_status_json(&self) -> Option<String> {
        self.serve_json.lock().unwrap().clone()
    }
    async fn serve_on(&self, h: u16, l: u16) -> Result<(), String> {
        self.log.lock().unwrap().push(format!("on {h} {l}"));
        Ok(())
    }
    async fn serve_off(&self, h: u16) -> Result<(), String> {
        self.log.lock().unwrap().push(format!("off {h}"));
        Ok(())
    }
}

#[tokio::test]
async fn serve_lifecycle() {
    let ts = FakeTs::default();
    let mut m = ServeManager::new(ts.clone()).with_port(9443);
    // never brings tailscale up: refuses while it is down, and runs no serve command
    assert!(m.enable(4000).await.unwrap_err().contains("turned off"));
    assert!(ts.log.lock().unwrap().is_empty());
    *ts.up.lock().unwrap() = true;
    // a stale entry from a crash is removed first; other ports untouched
    *ts.serve_json.lock().unwrap() = Some(r#"{"Web":{"m.ts.net:9443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:1"}}},"m.ts.net:443":{"Handlers":{"/":{"Proxy":"http://127.0.0.1:5199"}}}}}"#.into());
    m.enable(4000).await.unwrap();
    assert_eq!(*ts.log.lock().unwrap(), vec!["off 9443", "on 9443 4000"]);
    assert!(m.enabled());
    assert_eq!(m.tick().await, TickOutcome::Unchanged);
    // tailscale drops: the switch turns itself off, removes the serve entry, and stays off
    *ts.up.lock().unwrap() = false;
    assert_eq!(m.tick().await, TickOutcome::TurnedOff);
    assert!(!m.enabled());
    assert_eq!(ts.log.lock().unwrap().last().unwrap(), "off 9443");
    *ts.up.lock().unwrap() = true;
    assert_eq!(m.tick().await, TickOutcome::Unchanged);
    assert!(!m.enabled(), "stays off until the user re-enables it");
    m.enable(4000).await.unwrap();
    m.disable().await;
    assert_eq!(ts.log.lock().unwrap().last().unwrap(), "off 9443");
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(addr: std::net::SocketAddr, origin: Option<&str>) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://{addr}/v1/ws").into_client_request().unwrap();
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    tokio_tungstenite::connect_async(req).await.map(|x| x.0)
}
async fn recv(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(5), ws.next()).await.expect("timeout") {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(&t).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("closed: {other:?}"),
        }
    }
}
async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::text(v.to_string())).await.unwrap();
}

struct Echo {
    opened: Arc<Mutex<Vec<TerminalLaunch>>>,
}
impl TerminalBackend for Echo {
    fn open(&self, l: TerminalLaunch) -> Result<TerminalHandle, String> {
        self.opened.lock().unwrap().push(l);
        let (otx, orx) = mpsc::channel(16);
        let (itx, mut irx) = mpsc::channel(16);
        tokio::spawn(async move {
            let _ = otx.send(b"$ ".to_vec()).await;
            while let Some(i) = irx.recv().await {
                match i {
                    TermInput::Data(d) => {
                        let _ = otx.send(d).await;
                    }
                    TermInput::Resize(..) => {}
                    TermInput::Close => break,
                }
            }
        });
        Ok(TerminalHandle { output: orx, input: itx, scrollback: Some(5000) })
    }
}

async fn setup() -> (RemoteServer, Keys, Arc<Mutex<Vec<TerminalLaunch>>>) {
    let keys = Keys::generate();
    let opened = Arc::new(Mutex::new(vec![]));
    let mut cfg = ServerConfig::new(keys.public_key(), "dsk1", vec![ORIGIN.into()]);
    cfg.backend = Some(Arc::new(Echo { opened: opened.clone() }));
    cfg.auth_timeout = Duration::from_secs(2);
    (RemoteServer::start(cfg, 0).await.unwrap(), keys, opened)
}

async fn auth(ws: &mut Ws, keys: &Keys) -> Value {
    let c = recv(ws).await;
    assert_eq!(c["t"], "challenge");
    let ev = sign(keys, c["nonce"].as_str().unwrap(), c["connId"].as_str().unwrap(), "dsk1", "connect", None);
    send(ws, json!({"t":"auth","event":ev})).await;
    recv(ws).await
}

#[tokio::test]
async fn listener_is_loopback_and_origin_checked() {
    let (srv, _k, _) = setup().await;
    assert!(srv.addr.ip().is_loopback());
    assert!(connect(srv.addr, Some("https://evil.example")).await.is_err());
    assert!(connect(srv.addr, None).await.is_err()); // browsers always send Origin
    assert!(connect(srv.addr, Some(ORIGIN)).await.is_ok());
    srv.shutdown();
}

#[tokio::test]
async fn auth_flow_and_rejections() {
    let (srv, keys, _) = setup().await;
    // good
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    let r = auth(&mut ws, &keys).await;
    assert_eq!((r["t"].as_str(), r["desktopId"].as_str(), r["terminals"].as_bool()), (Some("ready"), Some("dsk1"), Some(false)));
    send(&mut ws, json!({"t":"ping"})).await;
    assert_eq!(recv(&mut ws).await["t"], "pong");
    // frames before auth are refused
    let mut ws2 = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    recv(&mut ws2).await;
    send(&mut ws2, json!({"t":"ping"})).await;
    assert_eq!(recv(&mut ws2).await["code"], "auth_required");
    // someone else's key
    let mut ws3 = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut ws3, &Keys::generate()).await["code"], "wrong_member");
    // a signature captured from one connection is useless on another
    let mut a = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    let mut b = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    let ca = recv(&mut a).await;
    let cb = recv(&mut b).await;
    let ev = sign(&keys, ca["nonce"].as_str().unwrap(), ca["connId"].as_str().unwrap(), "dsk1", "connect", None);
    send(&mut b, json!({"t":"auth","event":ev})).await;
    assert_eq!(recv(&mut b).await["code"], "wrong_connection");
    let _ = cb;
    // signed for a different desktop
    let mut c = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    let cc = recv(&mut c).await;
    let ev = sign(&keys, cc["nonce"].as_str().unwrap(), cc["connId"].as_str().unwrap(), "dsk-other", "connect", None);
    send(&mut c, json!({"t":"auth","event":ev})).await;
    assert_eq!(recv(&mut c).await["code"], "wrong_desktop");
    // never authenticates: closed after the auth timeout
    let mut d = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    recv(&mut d).await;
    assert_eq!(recv(&mut d).await["code"], "auth_timeout");
    srv.shutdown();
}

#[tokio::test]
async fn terminals_off_by_default_then_fresh_signature_per_launch_cap_and_kill() {
    let (srv, keys, opened) = setup().await;
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut ws, &keys).await["t"], "ready");
    // off by default
    send(&mut ws, json!({"t":"term.challenge","launchId":"launch-0001"})).await;
    assert_eq!(recv(&mut ws).await["code"], "terminals_disabled");
    srv.set_terminals_enabled(true);
    let b64 = base64::engine::general_purpose::STANDARD;
    async fn launch(ws: &mut Ws, keys: &Keys, lid: &str, signed_purpose: &str, session: Option<&str>) -> Value {
        send(ws, json!({"t":"term.challenge","launchId":lid})).await;
        let c = recv(ws).await;
        assert_eq!(c["purpose"], format!("terminal:{lid}"));
        let ev = sign(keys, c["nonce"].as_str().unwrap(), c["connId"].as_str().unwrap(), "dsk1", signed_purpose, None);
        send(ws, json!({"t":"term.open","launchId":lid,"cols":100,"rows":30,"session":session,"event":ev})).await;
        recv(ws).await
    }
    // a signature for the wrong purpose (a connection signature) can't open a terminal
    let r = launch(&mut ws, &keys, "launch-0001", "connect", None).await;
    assert_eq!(r["code"], "wrong_purpose");
    // another launch's signature can't be used for this id
    let r = launch(&mut ws, &keys, "launch-0001", "terminal:launch-9999", None).await;
    assert_eq!(r["code"], "wrong_purpose");
    // proper launch with a herdr session name
    let r = launch(&mut ws, &keys, "launch-0001", "terminal:launch-0001", Some("work")).await;
    assert_eq!(r["t"], "term.opened");
    assert_eq!(opened.lock().unwrap()[0].session.as_deref(), Some("work"));
    let first = opened.lock().unwrap()[0].clone();
    assert_eq!((first.cols, first.rows), (100, 30));
    let out = recv(&mut ws).await;
    assert_eq!(out["t"], "term.out");
    assert_eq!(b64.decode(out["data"].as_str().unwrap()).unwrap(), b"$ ");
    send(&mut ws, json!({"t":"term.in","id":"launch-0001","data":b64.encode(b"ls\n")})).await;
    assert_eq!(b64.decode(recv(&mut ws).await["data"].as_str().unwrap()).unwrap(), b"ls\n");
    // bad session names are refused (no shell metacharacters reach the backend)
    let r = launch(&mut ws, &keys, "launch-0002", "terminal:launch-0002", Some("x; rm -rf ~")).await;
    assert_eq!(r["code"], "bad_session");
    // concurrency cap = 3
    for i in 2..=3 {
        let lid = format!("launch-000{i}");
        let r = launch(&mut ws, &keys, &lid, &format!("terminal:{lid}"), None).await;
        assert_eq!(r["t"], "term.opened");
        recv(&mut ws).await; // prompt
    }
    let r = launch(&mut ws, &keys, "launch-0004", "terminal:launch-0004", None).await;
    assert_eq!(r["code"], "too_many_terminals");
    assert_eq!(srv.open_terminals(), 3);
    // kill list / turning the toggle off closes everything
    srv.set_terminals_enabled(false);
    let mut closed = 0;
    while closed < 3 {
        if recv(&mut ws).await["t"] == "term.closed" {
            closed += 1;
        }
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(srv.open_terminals(), 0);
    srv.shutdown();
}

#[tokio::test]
async fn real_pty_backend_runs_a_shell_with_a_clean_environment() {
    use buzz_remote_access::{PtyBackend, TerminalMode};
    std::env::set_var("BUZZ_SECRET_TEST", "hunter2");
    let mut be = PtyBackend::new(TerminalMode::Shell);
    be.shell = Some("/bin/sh".into());
    let mut h = be.open(TerminalLaunch { launch_id: "launch-0001".into(), cols: 80, rows: 24, session: None, host: None }).unwrap();
    h.input.send(TermInput::Data(b"echo ready-$((6*7)); env | grep -c BUZZ_SECRET; exit\n".to_vec())).await.unwrap();
    let mut all = Vec::new();
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < end {
        match tokio::time::timeout(Duration::from_millis(500), h.output.recv()).await {
            Ok(Some(c)) => all.extend(c),
            Ok(None) => break,
            Err(_) => {}
        }
    }
    let s = String::from_utf8_lossy(&all);
    assert!(s.contains("ready-42"), "{s}");
    assert!(!s.contains("hunter2"));
    assert!(s.lines().any(|l| l.trim() == "0"), "app env must not leak into the terminal: {s}");
}

#[test]
fn herdr_session_list_parsing() {
    let out = "name status directory socket\ndefault running /Users/x/.config/herdr /s/herdr.sock\nbuzz stopped /d /s2\nbad;name running /d /s\n";
    let l = buzz_remote_access::pty::parse_herdr_sessions(out);
    assert_eq!(l.iter().map(|s| (s.name.as_str(), s.status.as_str())).collect::<Vec<_>>(), vec![("default", "running"), ("buzz", "stopped")]);
}

#[tokio::test]
async fn sessions_list_needs_auth_and_terminals_on() {
    let (srv, keys, _) = setup().await;
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut ws, &keys).await["t"], "ready");
    send(&mut ws, json!({"t":"sessions.list"})).await;
    assert_eq!(recv(&mut ws).await["code"], "terminals_disabled");
    srv.set_terminals_enabled(true);
    send(&mut ws, json!({"t":"sessions.list"})).await;
    let r = recv(&mut ws).await;
    assert_eq!((r["t"].as_str(), r["sessions"].as_array().map(|a| a.len())), (Some("sessions"), Some(0)));
    srv.shutdown();
}

// ── agent management over the direct connection ──────────────────────────────────────────────────────────────
struct FakeMgmt {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}
impl buzz_remote_access::ManagementBackend for FakeMgmt {
    fn call(&self, op: String, params: Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, String>> + Send>> {
        self.calls.lock().unwrap().push((op.clone(), params.clone()));
        Box::pin(async move {
            match op.as_str() {
                "agents.list" => Ok(json!({"agents":[{"name":"a"}]})),
                "agent.slow" => {
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    Ok(json!({}))
                }
                _ => Err("nope".into()),
            }
        })
    }
}

async fn setup_mgmt(ttl: Duration) -> (RemoteServer, Keys, Arc<Mutex<Vec<(String, Value)>>>) {
    let keys = Keys::generate();
    let calls = Arc::new(Mutex::new(vec![]));
    let mut cfg = ServerConfig::new(keys.public_key(), "dsk1", vec![ORIGIN.into()]);
    cfg.mgmt = Some(Arc::new(FakeMgmt { calls: calls.clone() }));
    cfg.session_ttl = ttl;
    (RemoteServer::start(cfg, 0).await.unwrap(), keys, calls)
}

#[tokio::test]
async fn management_is_off_by_default_needs_auth_and_checks_the_switch_on_every_request() {
    let (srv, keys, calls) = setup_mgmt(Duration::from_secs(3600)).await;
    // unauthenticated connections can't reach it
    let mut anon = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    recv(&mut anon).await;
    send(&mut anon, json!({"t":"mgmt","id":"r1","op":"agents.list"})).await;
    assert_eq!(recv(&mut anon).await["code"], "auth_required");
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    let ready = auth(&mut ws, &keys).await;
    assert_eq!((ready["agents"].as_bool(), ready["sessionTtlS"].as_u64()), (Some(false), Some(3600)));
    send(&mut ws, json!({"t":"mgmt","id":"r1","op":"agents.list"})).await;
    let r = recv(&mut ws).await;
    assert_eq!((r["t"].as_str(), r["id"].as_str(), r["ok"].as_bool(), r["error"].as_str()), (Some("mgmt.result"), Some("r1"), Some(false), Some("agents_disabled")));
    assert!(calls.lock().unwrap().is_empty(), "nothing reaches the app while the switch is off");
    srv.set_agents_enabled(true);
    send(&mut ws, json!({"t":"mgmt","id":"r2","op":"agents.list","params":{"x":1}})).await;
    let r = recv(&mut ws).await;
    assert_eq!((r["ok"].as_bool(), r["result"]["agents"][0]["name"].as_str()), (Some(true), Some("a")));
    assert_eq!(calls.lock().unwrap()[0], ("agents.list".to_string(), json!({"x":1})));
    // app-side errors come back as errors
    send(&mut ws, json!({"t":"mgmt","id":"r3","op":"agent.bogus"})).await;
    assert_eq!(recv(&mut ws).await["error"], "nope");
    // malformed op / id are refused before the app sees them
    for bad in [json!({"t":"mgmt","id":"r4","op":"a b"}), json!({"t":"mgmt","id":"","op":"agents.list"}), json!({"t":"mgmt","id":"r5","op":"x".repeat(200)})] {
        send(&mut ws, bad).await;
        assert_eq!(recv(&mut ws).await["error"], "bad_request");
    }
    assert_eq!(calls.lock().unwrap().len(), 2);
    // turning the switch off while a request runs withholds its result
    send(&mut ws, json!({"t":"mgmt","id":"r6","op":"agent.slow"})).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    srv.set_agents_enabled(false);
    let r = recv(&mut ws).await;
    assert_eq!((r["id"].as_str(), r["error"].as_str()), (Some("r6"), Some("agents_disabled")));
    srv.shutdown();
}

#[tokio::test]
async fn signature_lifetime_is_capped_at_24h_and_expiry_closes_the_connection() {
    let (srv, keys, _) = setup_mgmt(Duration::from_secs(48 * 3600)).await;
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut ws, &keys).await["sessionTtlS"], 24 * 3600);
    srv.shutdown();
    let (srv, keys, _) = setup_mgmt(Duration::from_secs(1)).await;
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut ws, &keys).await["sessionTtlS"], 1);
    // a connection authenticated after the lifetime is changed uses the new value
    srv.set_session_ttl(Duration::from_secs(7200));
    assert_eq!(recv(&mut ws).await["code"], "session_expired");
    let mut ws2 = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut ws2, &keys).await["sessionTtlS"], 7200);
    srv.shutdown();
}

// ── terminals: Buzz Term settings and the per-launch host ────────────────────────────────────────────────────
#[tokio::test]
async fn launch_carries_the_per_launch_host_and_opened_reports_scrollback() {
    let (srv, keys, opened) = setup().await;
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    auth(&mut ws, &keys).await;
    srv.set_terminals_enabled(true);
    for (lid, host, want) in [("launch-0001", Some("herdr"), Some("herdr")), ("launch-0002", Some("bogus"), None), ("launch-0003", None, None)] {
        send(&mut ws, json!({"t":"term.challenge","launchId":lid})).await;
        let c = recv(&mut ws).await;
        let ev = sign(&keys, c["nonce"].as_str().unwrap(), c["connId"].as_str().unwrap(), "dsk1", &format!("terminal:{lid}"), None);
        send(&mut ws, json!({"t":"term.open","launchId":lid,"cols":80,"rows":24,"host":host,"event":ev})).await;
        let r = recv(&mut ws).await;
        assert_eq!((r["t"].as_str(), r["scrollback"].as_u64()), (Some("term.opened"), Some(5000)));
        recv(&mut ws).await; // prompt
        assert_eq!(opened.lock().unwrap().last().unwrap().host.as_deref(), want);
    }
    srv.shutdown();
}

#[tokio::test]
async fn pty_backend_honors_default_directory_and_shell_and_falls_back_safely() {
    use buzz_remote_access::pty::{resolve_cwd, resolve_shell};
    use buzz_remote_access::{PtyBackend, TerminalMode};
    let dir = std::fs::canonicalize(std::env::temp_dir()).unwrap();
    // real PTY started in the configured directory with the configured shell
    let mut be = PtyBackend::new(TerminalMode::Shell);
    be.shell = Some("/bin/sh".into());
    be.cwd = Some(dir.clone());
    be.scrollback = Some(2500);
    let mut h = be.open(TerminalLaunch { launch_id: "launch-0001".into(), cols: 80, rows: 24, session: None, host: None }).unwrap();
    assert_eq!(h.scrollback, Some(2500));
    h.input.send(TermInput::Data(b"echo cwd-is-$(pwd -P); echo shell-is-$0; exit\n".to_vec())).await.unwrap();
    let mut all = Vec::new();
    let end = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < end {
        match tokio::time::timeout(Duration::from_millis(500), h.output.recv()).await {
            Ok(Some(c)) => all.extend(c),
            Ok(None) => break,
            Err(_) => {}
        }
    }
    let s = String::from_utf8_lossy(&all);
    assert!(s.contains(&format!("cwd-is-{}", dir.display())), "{s}");
    assert!(s.contains("shell-is--sh") || s.contains("shell-is-/bin/sh") || s.contains("shell-is-sh"), "{s}");
    // pure resolution rules
    let home = Some(PathBuf::from("/Users/x"));
    assert_eq!(resolve_cwd(Some(&PathBuf::from("/definitely/not/here")), home.clone()), home);
    assert_eq!(resolve_cwd(None, home.clone()), home);
    assert_eq!(resolve_cwd(Some(&dir), home.clone()), Some(dir.clone()));
    assert_eq!(resolve_cwd(Some(&PathBuf::from("relative/dir")), home.clone()), home);
    assert_eq!(resolve_shell(Some(&PathBuf::from("zsh")), Some("/bin/bash".into())), PathBuf::from("/bin/bash")); // not absolute
    assert_eq!(resolve_shell(Some(&PathBuf::from("/no/such/shell")), None), PathBuf::from("/bin/zsh"));
    assert_eq!(resolve_shell(Some(&PathBuf::from("/bin/sh")), Some("/bin/bash".into())), PathBuf::from("/bin/sh"));
    assert_eq!(resolve_shell(Some(&PathBuf::from("/etc/hosts")), Some("/bin/bash".into())), PathBuf::from("/bin/bash")); // not executable
}

#[tokio::test]
async fn terminal_signature_window_allows_unsigned_launches_until_it_expires() {
    let keys = Keys::generate();
    let opened = Arc::new(Mutex::new(vec![]));
    let mut cfg = ServerConfig::new(keys.public_key(), "dsk1", vec![ORIGIN.into()]);
    cfg.backend = Some(Arc::new(Echo { opened: opened.clone() }));
    cfg.term_session_ttl = Duration::from_secs(2);
    let srv = RemoteServer::start(cfg, 0).await.unwrap();
    let mut ws = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    let ready = auth(&mut ws, &keys).await;
    assert_eq!(ready["termSessionTtlS"], 2);
    assert!(ready["signedUntil"].as_u64().unwrap() > 0);
    srv.set_terminals_enabled(true);
    // no signature yet: refused
    send(&mut ws, json!({"t":"term.open","launchId":"launch-0001","cols":80,"rows":24})).await;
    assert_eq!(recv(&mut ws).await["code"], "term_signature_required");
    // signed launch starts the window
    send(&mut ws, json!({"t":"term.challenge","launchId":"launch-0001"})).await;
    let c = recv(&mut ws).await;
    let ev = sign(&keys, c["nonce"].as_str().unwrap(), c["connId"].as_str().unwrap(), "dsk1", "terminal:launch-0001", None);
    send(&mut ws, json!({"t":"term.open","launchId":"launch-0001","cols":80,"rows":24,"event":ev})).await;
    let r = recv(&mut ws).await;
    assert_eq!(r["t"], "term.opened");
    assert!(r["termSignedUntil"].as_u64().unwrap() > 0);
    recv(&mut ws).await; // prompt
    // inside the window: no signature needed
    send(&mut ws, json!({"t":"term.open","launchId":"launch-0002","cols":80,"rows":24})).await;
    assert_eq!(recv(&mut ws).await["t"], "term.opened");
    recv(&mut ws).await;
    // after it ends: a fresh signature is required again
    tokio::time::sleep(Duration::from_millis(3100)).await;
    send(&mut ws, json!({"t":"term.open","launchId":"launch-0003","cols":80,"rows":24})).await;
    assert_eq!(recv(&mut ws).await["code"], "term_signature_required");
    // the terminal lifetime is changeable at runtime (clamped to 24 h)
    srv.set_terminal_session_ttl(Duration::from_secs(48 * 3600));
    let mut ws2 = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut ws2, &keys).await["termSessionTtlS"], 24 * 3600);
    srv.shutdown();
}

#[tokio::test]
async fn terminals_survive_a_dropped_connection_then_attach_replays_and_grace_reaps() {
    let keys = Keys::generate();
    let opened = Arc::new(Mutex::new(vec![]));
    let mut cfg = ServerConfig::new(keys.public_key(), "dsk1", vec![ORIGIN.into()]);
    cfg.backend = Some(Arc::new(Echo { opened: opened.clone() }));
    cfg.detach_grace = Duration::from_millis(600);
    let srv = RemoteServer::start(cfg, 0).await.unwrap();
    srv.set_terminals_enabled(true);
    let b64 = base64::engine::general_purpose::STANDARD;
    async fn signed(ws: &mut Ws, keys: &Keys, lid: &str, frame: Value) -> Value {
        send(ws, json!({"t":"term.challenge","launchId":lid})).await;
        let c = recv(ws).await;
        let ev = sign(keys, c["nonce"].as_str().unwrap(), c["connId"].as_str().unwrap(), "dsk1", &format!("terminal:{lid}"), None);
        let mut f = frame;
        f["event"] = ev.into();
        send(ws, f).await;
        recv(ws).await
    }
    let mut a = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    auth(&mut a, &keys).await;
    for lid in ["tab-0001", "tab-0002"] {
        let r = signed(&mut a, &keys, lid, json!({"t":"term.open","launchId":lid,"cols":80,"rows":24})).await;
        assert_eq!(r["t"], "term.opened");
        recv(&mut a).await; // prompt
    }
    send(&mut a, json!({"t":"term.in","id":"tab-0001","data":b64.encode(b"hello-1\n")})).await;
    recv(&mut a).await;
    drop(a); // page refresh
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(srv.detached_terminals(), 2);
    assert_eq!(srv.open_terminals(), 2);

    let mut b = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    auth(&mut b, &keys).await;
    send(&mut b, json!({"t":"term.list"})).await;
    let l = recv(&mut b).await;
    assert_eq!(l["t"], "term.list");
    let mut ids: Vec<&str> = l["sessions"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    ids.sort();
    assert_eq!(ids, ["tab-0001", "tab-0002"]);
    // attach needs a terminal signature (or an open window)
    send(&mut b, json!({"t":"term.attach","id":"tab-0001"})).await;
    assert_eq!(recv(&mut b).await["code"], "term_signature_required");
    // input to a terminal this connection doesn't drive is ignored
    let r = signed(&mut b, &keys, "tab-0001", json!({"t":"term.attach","id":"tab-0001"})).await;
    assert_eq!(r["t"], "term.attached");
    let rep = recv(&mut b).await;
    assert_eq!(rep["replay"], true);
    assert_eq!(b64.decode(rep["data"].as_str().unwrap()).unwrap(), b"$ hello-1\n");
    send(&mut b, json!({"t":"term.in","id":"tab-0001","data":b64.encode(b"x\n")})).await;
    assert_eq!(b64.decode(recv(&mut b).await["data"].as_str().unwrap()).unwrap(), b"x\n");
    // the second one was not attached: reaped after the grace period
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(srv.detached_terminals(), 0);
    assert_eq!(srv.open_terminals(), 1);
    // closing the tab kills it
    send(&mut b, json!({"t":"term.close","id":"tab-0001"})).await;
    assert_eq!(recv(&mut b).await["t"], "term.closed");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(srv.open_terminals(), 0);
    // a different member can't authenticate at all, so can't list/attach (auth is member-only)
    let mut c = connect(srv.addr, Some(ORIGIN)).await.unwrap();
    assert_eq!(auth(&mut c, &Keys::generate()).await["code"], "wrong_member");
    srv.shutdown();
}
