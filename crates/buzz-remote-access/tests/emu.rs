//! The web terminal path end to end: real PTY -> alacritty emulator -> row/span frames over the websocket,
//! with the per-launch signature, one-frame-in-flight credit, resize/scroll/mouse/focus/detach/close.
use std::sync::Arc;
use std::time::Duration;

use buzz_remote_access::{PtyBackend, RemoteServer, ServerConfig, TerminalMode};
use futures_util::{SinkExt, StreamExt};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn sign(keys: &Keys, nonce: &str, conn: &str, purpose: &str) -> String {
    EventBuilder::new(Kind::Custom(22242), "")
        .tags([
            Tag::parse(["challenge", nonce]).unwrap(),
            Tag::parse(["conn", conn]).unwrap(),
            Tag::parse(["desktop", "dsk1"]).unwrap(),
            Tag::parse(["purpose", purpose]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap()
        .as_json()
}

async fn recv(ws: &mut Ws) -> Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(8), ws.next()).await.expect("timeout") {
            Some(Ok(Message::Text(t))) => return serde_json::from_str(&t).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("closed: {other:?}"),
        }
    }
}
async fn send(ws: &mut Ws, v: Value) {
    ws.send(Message::text(v.to_string())).await.unwrap();
}

fn screen_text(rows: &mut Vec<String>, frame: &Value) {
    let h = frame["viewport"]["screenLines"].as_u64().unwrap() as usize;
    if frame["full"].as_bool().unwrap() || rows.len() != h {
        *rows = vec![String::new(); h];
    }
    for r in frame["rows"].as_array().unwrap() {
        let mut line = String::new();
        for s in r["spans"].as_array().unwrap() {
            for c in s["clusters"].as_array().unwrap() {
                let col = c["column"].as_u64().unwrap() as usize;
                while line.chars().count() < col {
                    line.push(' ');
                }
                line.push_str(c["text"].as_str().unwrap());
            }
        }
        rows[r["line"].as_u64().unwrap() as usize] = line;
    }
}

struct Term {
    ws: Ws,
    sub: String,
    rows: Vec<String>,
    frames: Vec<Value>,
    last_frame: Option<Value>,
}

impl Term {
    /// Pump messages until `done` is true on the reconstructed screen; acks every frame like the real client.
    async fn pump_until(&mut self, what: &str, done: impl Fn(&[String]) -> bool) {
        let end = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < end {
            if done(&self.rows) {
                return;
            }
            let m = recv(&mut self.ws).await;
            if m["t"] == "term.msg" && m["msg"]["type"] == "frame" {
                let f = m["msg"]["payload"].clone();
                screen_text(&mut self.rows, &f);
                send(&mut self.ws, json!({"t":"emu.ack","id":"launch-emu01","sub":f["subscriptionId"],"seq":f["sequence"]})).await;
                self.last_frame = Some(f.clone());
                self.frames.push(f);
            }
        }
        panic!("never saw {what}; screen: {:#?}", self.rows);
    }
}

async fn open(shell: &str) -> (RemoteServer, Term) {
    let keys = Keys::generate();
    let mut be = PtyBackend::new(TerminalMode::Shell);
    be.shell = Some(shell.into());
    be.scrollback = Some(2000);
    let mut cfg = ServerConfig::new(keys.public_key(), "dsk1", vec!["https://pwa.example".into()]);
    cfg.backend = Some(Arc::new(be));
    let srv = RemoteServer::start(cfg, 0).await.unwrap();
    srv.set_terminals_enabled(true);
    let mut req = format!("ws://{}/v1/ws", srv.addr).into_client_request().unwrap();
    req.headers_mut().insert("origin", "https://pwa.example".parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    let c = recv(&mut ws).await;
    send(&mut ws, json!({"t":"auth","event":sign(&keys, c["nonce"].as_str().unwrap(), c["connId"].as_str().unwrap(), "connect")})).await;
    assert_eq!(recv(&mut ws).await["t"], "ready");
    send(&mut ws, json!({"t":"term.challenge","launchId":"launch-emu01"})).await;
    let c = recv(&mut ws).await;
    let ev = sign(&keys, c["nonce"].as_str().unwrap(), c["connId"].as_str().unwrap(), "terminal:launch-emu01");
    send(&mut ws, json!({"t":"term.open","launchId":"launch-emu01","cols":60,"rows":12,"emu":true,"event":ev})).await;
    let mut early = vec![];
    let opened = loop {
        let m = recv(&mut ws).await;
        if m["t"] == "term.opened" {
            break m;
        }
        early.push(m);
    };
    assert_eq!(opened["emu"], true);
    assert_eq!(opened["viewport"]["columns"], 60);
    let sub = opened["subscriptionId"].as_str().unwrap().to_string();
    let mut t = Term { ws, sub, rows: vec![], frames: vec![], last_frame: None };
    // the bootstrap snapshot may arrive before `term.opened`; the client buffers it, so do we
    for m in early {
        if m["t"] == "term.msg" {
            let f = m["msg"]["payload"].clone();
            assert_eq!(f["subscriptionId"], t.sub);
            screen_text(&mut t.rows, &f);
            send(&mut t.ws, json!({"t":"emu.ack","id":"launch-emu01","sub":f["subscriptionId"],"seq":f["sequence"]})).await;
        }
    }
    (srv, t)
}

#[tokio::test]
async fn real_shell_output_arrives_as_styled_cell_frames_and_input_round_trips() {
    let (srv, mut t) = open("/bin/sh").await;
    send(&mut t.ws, json!({"t":"emu.input","id":"launch-emu01","data":"printf '\\033[31mred\\033[0m-%s\\n' $((6*7))\n"})).await;
    t.pump_until("rendered red-42", |r| r.iter().any(|l| l.contains("red-42"))).await;
    // colour survives: a span with a non-default fg exists for the "red" text
    let styled = t.frames.iter().flat_map(|f| f["rows"].as_array().unwrap().clone()).flat_map(|r| r["spans"].as_array().unwrap().clone()).any(|s| {
        s["clusters"].as_array().unwrap().iter().any(|c| c["text"].as_str().unwrap_or("").starts_with('r')) && s["style"]["fg"].as_u64().unwrap_or(0) != 0
    });
    assert!(styled, "colour must be carried in span styles");
    assert!(t.last_frame.as_ref().unwrap()["cursor"]["visible"].is_boolean());
    srv.shutdown();
}

#[tokio::test]
async fn resize_reply_scroll_mouse_focus_and_unknown_ids_behave() {
    let (srv, mut t) = open("/bin/sh").await;
    send(&mut t.ws, json!({"t":"emu.resize","id":"launch-emu01","cols":80,"rows":20,"rid":"r1"})).await;
    let mut reply = None;
    while reply.is_none() {
        let m = recv(&mut t.ws).await;
        if m["t"] == "emu.reply" {
            reply = Some(m);
        }
    }
    let reply = reply.unwrap();
    assert_eq!(reply["ok"], true);
    let vp = reply["result"].clone();
    assert_eq!(vp["columns"], 80);
    assert_eq!(vp["screenLines"], 20);
    // the renderer confirms it installed the new viewport -> frames flow again
    send(&mut t.ws, json!({"t":"emu.viewport_ready","id":"launch-emu01","sub":t.sub,"viewport":vp})).await;
    // generate scrollback, then scroll into it
    send(&mut t.ws, json!({"t":"emu.input","id":"launch-emu01","data":"i=0; while [ $i -lt 60 ]; do echo line-$i; i=$((i+1)); done\n"})).await;
    t.pump_until("line-59", |r| r.iter().any(|l| l.contains("line-59"))).await;
    send(&mut t.ws, json!({"t":"emu.scroll","id":"launch-emu01","lines":-10})).await;
    t.pump_until("scrolled up", |r| !r.iter().any(|l| l.contains("line-59"))).await;
    // typing returns to the live edge
    send(&mut t.ws, json!({"t":"emu.input","id":"launch-emu01","data":"echo back\n"})).await;
    t.pump_until("back at the bottom", |r| r.iter().any(|l| l.contains("back"))).await;
    // mouse tracking is off in a plain shell: reported as not handled
    send(&mut t.ws, json!({"t":"emu.mouse","id":"launch-emu01","rid":"m1","req":{"button":"left","action":"press","column":2,"row":2}})).await;
    loop {
        let m = recv(&mut t.ws).await;
        if m["t"] == "emu.reply" && m["rid"] == "m1" {
            assert_eq!(m["ok"], true);
            assert_eq!(m["result"], false);
            break;
        }
    }
    send(&mut t.ws, json!({"t":"emu.mouse","id":"launch-emu01","rid":"m2","req":{"button":"banana","action":"press","column":2,"row":2}})).await;
    loop {
        let m = recv(&mut t.ws).await;
        if m["t"] == "emu.reply" && m["rid"] == "m2" {
            assert_eq!(m["ok"], false);
            break;
        }
    }
    send(&mut t.ws, json!({"t":"emu.focus","id":"launch-emu01","focused":true})).await;
    // unknown terminal id is an error reply, not a hang
    send(&mut t.ws, json!({"t":"emu.input","id":"nope","data":"x"})).await;
    loop {
        let m = recv(&mut t.ws).await;
        if m["t"] == "emu.reply" && m["id"] == "nope" {
            assert_eq!(m["ok"], false);
            break;
        }
    }
    srv.shutdown();
}

#[tokio::test]
async fn at_most_one_frame_is_in_flight_until_acked_and_detach_then_attach_resnapshots() {
    let (srv, mut t) = open("/bin/sh").await;
    // no acks from here on: output must coalesce into at most one further frame, not stream unboundedly
    send(&mut t.ws, json!({"t":"emu.input","id":"launch-emu01","data":"i=0; while [ $i -lt 300 ]; do echo spam-$i; i=$((i+1)); done\n"})).await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let mut frames = 0;
    while let Ok(Some(Ok(Message::Text(x)))) = tokio::time::timeout(Duration::from_millis(300), t.ws.next()).await {
        let v: Value = serde_json::from_str(&x).unwrap();
        if v["t"] == "term.msg" && v["msg"]["type"] == "frame" {
            frames += 1;
        }
    }
    assert!(frames <= 1, "credit must hold frames back: saw {frames}");
    // detach + attach: a fresh full snapshot on a new subscription
    send(&mut t.ws, json!({"t":"emu.detach","id":"launch-emu01","sub":t.sub})).await;
    send(&mut t.ws, json!({"t":"emu.attach","id":"launch-emu01","rid":"a1"})).await;
    let mut got_snapshot = false;
    let mut new_sub = None;
    for _ in 0..10 {
        let m = recv(&mut t.ws).await;
        if m["t"] == "emu.reply" && m["rid"] == "a1" {
            assert_eq!(m["ok"], true);
            new_sub = m["result"]["subscriptionId"].as_str().map(str::to_string);
        }
        if m["t"] == "term.msg" && m["msg"]["type"] == "frame" && m["msg"]["payload"]["full"] == true && Some(m["msg"]["payload"]["subscriptionId"].as_str().unwrap().to_string()) != Some(t.sub.clone()) {
            got_snapshot = true;
        }
        if got_snapshot && new_sub.is_some() {
            break;
        }
    }
    assert!(got_snapshot && new_sub.is_some());
    // close ends the terminal
    send(&mut t.ws, json!({"t":"emu.close","id":"launch-emu01"})).await;
    loop {
        let m = recv(&mut t.ws).await;
        if m["t"] == "term.closed" {
            break;
        }
    }
    srv.shutdown();
}

#[tokio::test]
async fn full_screen_apps_use_the_alternate_screen_and_report_input_modes() {
    let (srv, mut t) = open("/bin/sh").await;
    // enable alt screen + bracketed paste + SGR mouse tracking + focus reporting like herdr/vim do
    send(&mut t.ws, json!({"t":"emu.input","id":"launch-emu01","data":"printf '\\033[?1049h\\033[?2004h\\033[?1000h\\033[?1006h\\033[?1004hALT-%s' $((6*7))\n"})).await;
    t.pump_until("alt screen text", |r| r.iter().any(|l| l.contains("ALT-42"))).await;
    let f = t.last_frame.clone().unwrap();
    assert_eq!(f["bracketedPaste"], true);
    assert_eq!(f["focusReporting"], true);
    assert_eq!(f["mouseReporting"], true);
    // with mouse reporting on, a press is forwarded to the PTY (handled = true)
    send(&mut t.ws, json!({"t":"emu.mouse","id":"launch-emu01","rid":"m1","req":{"button":"left","action":"press","column":3,"row":3}})).await;
    loop {
        let m = recv(&mut t.ws).await;
        if m["t"] == "term.msg" && m["msg"]["type"] == "frame" {
            let f = &m["msg"]["payload"];
            send(&mut t.ws, json!({"t":"emu.ack","id":"launch-emu01","sub":f["subscriptionId"],"seq":f["sequence"]})).await;
        }
        if m["t"] == "emu.reply" && m["rid"] == "m1" {
            assert_eq!(m["result"], true);
            break;
        }
    }
    srv.shutdown();
}
