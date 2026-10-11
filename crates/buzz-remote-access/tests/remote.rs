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
        Ok(TerminalHandle { output: orx, input: itx })
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
    let mut h = be.open(TerminalLaunch { launch_id: "launch-0001".into(), cols: 80, rows: 24, session: None }).unwrap();
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
