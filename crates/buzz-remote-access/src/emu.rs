//! Server-side terminal emulation for the web client.
//!
//! The desktop's Buzz Term runs PTY bytes through `buzz-terminal` (alacritty_terminal) and ships structured
//! row/span/cell frames to its renderer. This module runs the very same engine in the remote listener and pushes
//! the very same frames (`buzz_terminal::wire`) over the websocket, with the same one-frame-in-flight credit
//! (`FramePublisher`) so a slow phone cannot make the listener buffer unbounded output (the PTY simply blocks).
//!
//! Wire (all JSON text frames; `id` is the launch id):
//!   server -> {"t":"term.msg","id","msg":{"type":"frame"|"title"|"resetTitle"|"bell"|"exit","payload":..}}
//!   client -> {"t":"emu.input","id","data":<utf8 string>}            keystrokes / paste (the web client's strings)
//!   client -> {"t":"emu.resize","id","cols","rows","pw","ph","rid"} -> {"t":"emu.reply","id","rid","ok","result"}
//!   client -> {"t":"emu.viewport_ready","id","sub","viewport"} / {"t":"emu.ack","id","sub","seq"}
//!   client -> {"t":"emu.scroll","id","lines"} / {"t":"emu.mouse","id","rid","req"} / {"t":"emu.focus","id","focused"}
//!   client -> {"t":"emu.attach","id"}  (re-subscribe to a live terminal: fresh snapshot) -> emu.reply
//!   client -> {"t":"emu.detach","id","sub"} / {"t":"emu.close","id"}
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use buzz_terminal::damage::Encoder;
use buzz_terminal::mouse_report::{encode_mouse_event, MouseAction, MouseButton};
use buzz_terminal::scroll_sign::{scroll_by_dom_lines, DomLines};
use buzz_terminal::transport::{FramePublisher, OfferError, Publication, SubscriptionId};
use buzz_terminal::wheel_dispatch::{plan as plan_wheel, WheelAction};
use buzz_terminal::wire::{wire_publication, TerminalMessage, WireViewport};
use buzz_terminal::{Fences, SharedTerminal, Size, Terminal};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

use crate::server::{TermInput, TerminalHandle};

pub const MAX_EMU_INPUT: usize = 64 * 1024;

type R<T> = Result<T, String>;

pub fn clamp_dims(cols: u64, rows: u64) -> (u16, u16) {
    (cols.clamp(10, 500) as u16, rows.clamp(2, 200) as u16)
}

pub struct EmuSession {
    id: String,
    terminal: Arc<SharedTerminal>,
    publisher: Mutex<FramePublisher>,
    /// The attached connection's outbox; None while the session is detached (kept alive for a later term.attach).
    out: Mutex<Option<mpsc::Sender<Message>>>,
    /// Last title the program set (for term.list after a reconnect).
    title: Mutex<Option<String>>,
    /// Whether a websocket subscriber is currently attached (frames are only sent while true).
    attached: AtomicBool,
    input: mpsc::Sender<TermInput>,
    dims: Mutex<(u16, u16)>,
}

fn send_blocking_or_try(out: &mpsc::Sender<Message>, m: Message) -> bool {
    // Reader thread: block (backpressure to the PTY). Async task (ack/resize paths): never block the runtime.
    if tokio::runtime::Handle::try_current().is_ok() {
        out.try_send(m).is_ok()
    } else {
        out.blocking_send(m).is_ok()
    }
}

impl EmuSession {
    /// Start emulating `handle`'s output. Returns the session, the bootstrap frame's subscription id + viewport,
    /// and a receiver that fires when the PTY output ends (so the server can account and report `term.closed`).
    pub fn start(
        id: &str,
        handle: TerminalHandle,
        cols: u16,
        rows: u16,
        out: mpsc::Sender<Message>,
    ) -> R<(Arc<EmuSession>, SubscriptionId, WireViewport, oneshot::Receiver<()>)> {
        let scrollback = handle.scrollback.map(|n| (n as usize).clamp(100, 100_000)).unwrap_or(Size::default().scrollback);
        let size = Size { columns: cols as usize, screen_lines: rows as usize, scrollback };
        let (terminal, actions) = Terminal::new(size, Fences::default());
        let terminal = Arc::new(SharedTerminal::new(terminal));
        let publisher = Mutex::new(FramePublisher::new(terminal.lock().viewport()));
        let session = Arc::new(EmuSession {
            id: id.to_string(),
            terminal: terminal.clone(),
            publisher,
            out: Mutex::new(Some(out)),
            title: Mutex::new(None),
            attached: AtomicBool::new(false),
            input: handle.input.clone(),
            dims: Mutex::new((cols, rows)),
        });

        // Emulator replies (DSR, size queries) go back to the PTY even with no subscriber.
        let a_in = handle.input.clone();
        let a_session = Arc::downgrade(&session);
        std::thread::spawn(move || {
            while let Ok(action) = actions.recv() {
                let msg = match &action {
                    buzz_terminal::Action::Title(t) => Some(TerminalMessage::Title(t.clone())),
                    buzz_terminal::Action::ResetTitle => Some(TerminalMessage::ResetTitle),
                    buzz_terminal::Action::Bell => Some(TerminalMessage::Bell),
                    _ => None,
                };
                if let Some(m) = msg {
                    if let Some(s) = a_session.upgrade() {
                        if let Ok(mut t) = s.title.lock() {
                            match &m {
                                TerminalMessage::Title(x) => *t = Some(x.chars().take(200).collect()),
                                TerminalMessage::ResetTitle => *t = None,
                                _ => {}
                            }
                        }
                        if s.attached.load(Ordering::Acquire) {
                            let _ = s.send_msg(&m);
                        }
                    }
                    continue;
                }
                let (c, r) = a_session.upgrade().and_then(|s| s.dims.lock().ok().map(|d| *d)).unwrap_or((80, 24));
                if let Some(reply) = buzz_terminal::listener::reply(action, c, r, 0, 0) {
                    if a_in.blocking_send(TermInput::Data(reply.into_bytes())).is_err() {
                        break;
                    }
                }
            }
        });

        let mut output = handle.output;
        let (done_tx, done_rx) = oneshot::channel();
        let r_session = session.clone();
        std::thread::spawn(move || {
            let mut encoder = Encoder::new();
            let mut snap = Encoder::new();
            while let Some(chunk) = output.blocking_recv() {
                let t = &r_session.terminal;
                if t.is_closing() {
                    continue;
                }
                let mut more = t.feed(&chunk);
                while more && !t.is_closing() {
                    more = t.drain();
                }
                if t.is_closing() {
                    continue;
                }
                let needs_snapshot = r_session.publisher.lock().map(|p| p.requires_snapshot()).unwrap_or(false);
                let frame = if needs_snapshot { t.snapshot(&mut snap) } else { t.render(&mut encoder) };
                if frame.is_empty() {
                    continue;
                }
                let offered = r_session.publisher.lock().ok().map(|mut p| p.offer(frame));
                let publication = match offered {
                    Some(Ok(p)) => p,
                    Some(Err(OfferError::PendingFrameMustBeSnapshot)) => {
                        let s = t.snapshot(&mut snap);
                        r_session.publisher.lock().ok().and_then(|mut p| p.offer(s).ok().flatten())
                    }
                    None => None,
                };
                if let Some(p) = publication {
                    r_session.publish(p);
                }
            }
            let _ = r_session.send_msg(&TerminalMessage::Exit);
            let _ = done_tx.send(());
        });

        let (sub, viewport) = session.attach()?;
        Ok((session, sub, viewport, done_rx))
    }

    /// (Re)subscribe: a fresh bootstrap snapshot, no credit inherited from an earlier subscription.
    pub fn attach(&self) -> R<(SubscriptionId, WireViewport)> {
        let sub = SubscriptionId::new();
        let mut enc = Encoder::new();
        let snapshot = self.terminal.snapshot(&mut enc);
        let viewport = snapshot.viewport;
        let bootstrap = self
            .publisher
            .lock()
            .map_err(|e| e.to_string())?
            .attach(sub, snapshot)
            .map_err(|_| "terminal snapshot rejected".to_string())?;
        self.attached.store(true, Ordering::Release);
        self.publish(bootstrap);
        Ok((sub, viewport.into()))
    }

    fn send_msg(&self, m: &TerminalMessage) -> bool {
        let text = json!({"t":"term.msg","id":self.id,"msg":serde_json::to_value(m).unwrap_or(Value::Null)}).to_string();
        let Some(out) = self.out.lock().ok().and_then(|o| o.clone()) else { return false };
        send_blocking_or_try(&out, Message::text(text))
    }

    /// Point the session at a new connection's outbox (None = detach and drop the old outbox, frames stop).
    pub fn rebind(&self, out: Option<mpsc::Sender<Message>>) {
        if out.is_none() {
            self.attached.store(false, Ordering::Release);
        }
        if let Ok(mut o) = self.out.lock() {
            *o = out;
        }
    }

    pub fn title(&self) -> Option<String> {
        self.title.lock().ok().and_then(|t| t.clone())
    }

    pub fn dims(&self) -> (u16, u16) {
        self.dims.lock().map(|d| *d).unwrap_or((80, 24))
    }

    fn publish(&self, publication: Publication) {
        let sub = publication.subscription_id;
        let ok = wire_publication(publication).and_then(|mut m| {
            let (bp, fr, mr) = self.terminal.input_modes();
            m.bracketed_paste = bp;
            m.focus_reporting = fr;
            m.mouse_reporting = mr;
            if !self.attached.load(Ordering::Acquire) {
                return Err("detached".to_string());
            }
            if self.send_msg(&TerminalMessage::Frame(m)) { Ok(()) } else { Err("socket closed".to_string()) }
        });
        if ok.is_err() {
            self.attached.store(false, Ordering::Release);
            if let Ok(mut p) = self.publisher.lock() {
                p.fault(sub);
            }
        }
    }

    fn publish_viewport(&self) -> R<()> {
        let mut enc = Encoder::new();
        let p = self
            .publisher
            .lock()
            .map_err(|e| e.to_string())?
            .offer(self.terminal.snapshot(&mut enc))
            .map_err(|_| "terminal scroll snapshot rejected".to_string())?;
        if let Some(p) = p {
            self.publish(p);
        }
        Ok(())
    }

    pub async fn write(&self, bytes: Vec<u8>) -> R<()> {
        self.input.send(TermInput::Data(bytes)).await.map_err(|_| "terminal closed".to_string())
    }

    pub async fn handle_input(&self, data: &str) -> R<()> {
        if data.len() > MAX_EMU_INPUT {
            return Err("terminal input too large".into());
        }
        self.write(data.as_bytes().to_vec()).await?;
        // typing returns to the live edge, as on the desktop
        if self.terminal.scroll_to_bottom() {
            self.publish_viewport()?;
        }
        Ok(())
    }

    pub async fn resize(&self, cols: u16, rows: u16) -> R<WireViewport> {
        let size = Size { columns: cols as usize, screen_lines: rows as usize, scrollback: Size::default().scrollback };
        if let Ok(mut d) = self.dims.lock() {
            *d = (cols, rows);
        }
        let _ = self.input.send(TermInput::Resize(cols, rows)).await;
        let viewport = self.terminal.resize(size);
        let publication = {
            let mut p = self.publisher.lock().map_err(|e| e.to_string())?;
            p.resize_applied(viewport);
            let mut enc = Encoder::new();
            p.offer(self.terminal.snapshot(&mut enc)).map_err(|_| "terminal resize snapshot rejected".to_string())?
        };
        if let Some(p) = publication {
            self.publish(p);
        }
        Ok(viewport.into())
    }

    pub fn viewport_ready(&self, sub: &str, viewport: WireViewport) -> R<()> {
        let id = SubscriptionId::parse(sub)?;
        let p = self.publisher.lock().map_err(|e| e.to_string())?.viewport_ready(id, viewport.into());
        if let Some(p) = p {
            self.publish(p);
        }
        Ok(())
    }

    pub fn ack(&self, sub: &str, seq: u64) -> R<()> {
        let id = SubscriptionId::parse(sub)?;
        let p = self.publisher.lock().map_err(|e| e.to_string())?.acknowledge(id, seq);
        if let Some(p) = p {
            self.publish(p);
        }
        Ok(())
    }

    pub async fn scroll(&self, lines: i32) -> R<()> {
        let action = {
            let t = self.terminal.lock();
            plan_wheel(&t, DomLines(lines))
        };
        match action {
            WheelAction::Pty(bytes) => {
                if !bytes.is_empty() {
                    self.write(bytes).await?;
                }
                Ok(())
            }
            WheelAction::Scrollback(l) => {
                if !scroll_by_dom_lines(&self.terminal, l) {
                    return Ok(());
                }
                self.publish_viewport()
            }
        }
    }

    pub async fn mouse(&self, req: &Value) -> R<bool> {
        let button = match req["button"].as_str().unwrap_or("") {
            "left" => MouseButton::Left,
            "middle" => MouseButton::Middle,
            "right" => MouseButton::Right,
            "wheelUp" => MouseButton::WheelUp,
            "wheelDown" => MouseButton::WheelDown,
            _ => return Err("invalid mouse button".into()),
        };
        let action = match req["action"].as_str().unwrap_or("") {
            "press" => MouseAction::Press,
            "release" => MouseAction::Release,
            "move" => MouseAction::Move,
            _ => return Err("invalid mouse action".into()),
        };
        let mut mods = 0u8;
        if req["shift"].as_bool().unwrap_or(false) {
            mods |= 4;
        }
        if req["meta"].as_bool().unwrap_or(false) {
            mods |= 8;
        }
        if req["ctrl"].as_bool().unwrap_or(false) {
            mods |= 16;
        }
        let (col, row) = (req["column"].as_u64().unwrap_or(1) as usize, req["row"].as_u64().unwrap_or(1) as usize);
        let bytes = {
            let t = self.terminal.lock();
            match encode_mouse_event(&t, button, action, col, row, mods) {
                None => return Ok(false),
                Some(b) if b.is_empty() => return Ok(true),
                Some(b) => b,
            }
        };
        self.write(bytes).await?;
        Ok(true)
    }

    pub async fn focus(&self, focused: bool) -> R<()> {
        let (_, enabled, _) = self.terminal.input_modes();
        if enabled {
            self.write(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() }).await?;
        }
        Ok(())
    }

    pub fn detach(&self, sub: &str) -> R<()> {
        let id = SubscriptionId::parse(sub)?;
        if self.publisher.lock().map_err(|e| e.to_string())?.fault(id) {
            self.attached.store(false, Ordering::Release);
        }
        Ok(())
    }

    /// Stop publishing (the PTY itself is closed by the server via `TermInput::Close`).
    pub fn shutdown(&self) {
        self.attached.store(false, Ordering::Release);
        if let Ok(mut o) = self.out.lock() {
            *o = None;
        }
        if let Ok(mut p) = self.publisher.lock() {
            p.close();
        }
        self.terminal.begin_closing();
    }
}
