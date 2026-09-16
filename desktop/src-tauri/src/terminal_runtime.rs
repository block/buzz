//! Rust-owned PTY sessions and the typed Tauri transport for Buzz Substrate.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use buzz_terminal::context::{context_vars, GuiContext};
use buzz_terminal::damage::{Frame, Style};
use buzz_terminal::{Fences, SharedTerminal, Size, Terminal, Viewport};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use uuid::Uuid;

use crate::terminal_transport::{FramePublisher, OfferError, Publication, SubscriptionId};

mod scroll_sign;

use scroll_sign::{scroll_by_dom_lines, DomLines};

const MAX_LIVE_SESSIONS: usize = 20;
const MAX_INPUT_BYTES: usize = 1024 * 1024;

type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachRequest {
    /// Present when a renderer remounts onto an existing PTY-backed tab.
    session_id: Option<String>,
    channel_id: String,
    channel_name: String,
    thread_id: Option<String>,
    npub: String,
    relay_url: String,
    columns: u16,
    rows: u16,
    pixel_width: u16,
    pixel_height: u16,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireViewport {
    generation: u64,
    columns: usize,
    screen_lines: usize,
}

impl From<Viewport> for WireViewport {
    fn from(value: Viewport) -> Self {
        Self {
            generation: value.generation,
            columns: value.columns,
            screen_lines: value.screen_lines,
        }
    }
}

impl From<WireViewport> for Viewport {
    fn from(value: WireViewport) -> Self {
        Self {
            generation: value.generation,
            columns: value.columns,
            screen_lines: value.screen_lines,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachResponse {
    session_id: String,
    subscription_id: String,
    viewport: WireViewport,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireStyle {
    fg: u32,
    bg: u32,
    flags: u16,
}

impl From<Style> for WireStyle {
    fn from(value: Style) -> Self {
        Self {
            fg: value.fg,
            bg: value.bg,
            flags: value.flags,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireCluster {
    column: usize,
    text: String,
    width: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireSpan {
    style: WireStyle,
    clusters: Vec<WireCluster>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireRow {
    line: usize,
    wrapped: bool,
    spans: Vec<WireSpan>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WireCursor {
    line: usize,
    column: usize,
    visible: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrameMessage {
    subscription_id: String,
    sequence: u64,
    rows: Vec<WireRow>,
    cursor: WireCursor,
    full: bool,
    viewport: WireViewport,
    bracketed_paste: bool,
    focus_reporting: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub(crate) enum TerminalMessage {
    Frame(FrameMessage),
    Title(String),
    ResetTitle,
    Bell,
    Exit,
}

fn wire_publication(publication: Publication) -> Result<FrameMessage> {
    let frame = publication.frame;
    let rows = frame
        .rows
        .into_iter()
        .map(|row| {
            let spans = row
                .spans
                .into_iter()
                .map(|span| {
                    if !span.counts_are_consistent() {
                        return Err(
                            "terminal engine emitted an inconsistent cluster count".to_string()
                        );
                    }
                    let clusters = if span.cluster_count == 1 {
                        vec![WireCluster {
                            column: span.column,
                            text: span.text,
                            width: span.width,
                        }]
                    } else {
                        span.text
                            .chars()
                            .enumerate()
                            .map(|(index, ch)| WireCluster {
                                column: span.column + index * usize::from(span.width),
                                text: ch.to_string(),
                                width: span.width,
                            })
                            .collect()
                    };
                    Ok(WireSpan {
                        style: span.style.into(),
                        clusters,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(WireRow {
                line: row.line,
                wrapped: row.wrapped,
                spans,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(FrameMessage {
        subscription_id: publication.subscription_id.to_string(),
        sequence: publication.sequence,
        rows,
        cursor: WireCursor {
            line: frame.cursor.line,
            column: frame.cursor.column,
            visible: frame.cursor.visible,
        },
        full: frame.full,
        viewport: frame.viewport.into(),
        bracketed_paste: false,
        focus_reporting: false,
    })
}

fn feed_and_drain(terminal: &SharedTerminal, bytes: &[u8]) -> bool {
    let mut more = terminal.feed(bytes);
    let deferred = more;
    while more && !terminal.is_closing() {
        more = terminal.drain();
    }
    deferred
}

struct ReaderThread {
    handle: Option<JoinHandle<()>>,
    terminal: Arc<SharedTerminal>,
    stopping: Arc<AtomicBool>,
}

impl buzz_terminal::lifecycle::DrainingReader for ReaderThread {
    fn begin_closing(&self) {
        self.terminal.begin_closing();
    }

    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
    }

    fn join(mut self: Box<Self>) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct Session {
    id: Uuid,
    terminal: Arc<SharedTerminal>,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pty_size: Arc<Mutex<PtySize>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    reader: Option<Box<dyn buzz_terminal::lifecycle::DrainingReader + Send>>,
    publisher: Arc<Mutex<FramePublisher>>,
    channel: Arc<Mutex<Option<Channel<TerminalMessage>>>>,
}

impl Session {
    fn publish(&self, publication: Publication) {
        let subscription = publication.subscription_id;
        let sent = wire_publication(publication).and_then(|message| {
            let (bracketed_paste, focus_reporting) = self.terminal.input_modes();
            let mut message = message;
            message.bracketed_paste = bracketed_paste;
            message.focus_reporting = focus_reporting;
            self.channel
                .lock()
                .map_err(|e| e.to_string())?
                .as_ref()
                .ok_or_else(|| "terminal renderer detached".to_string())?
                .send(TerminalMessage::Frame(message))
                .map_err(|e| e.to_string())
        });
        if sent.is_err() {
            if let Ok(mut channel) = self.channel.lock() {
                *channel = None;
            }
            if let Ok(mut publisher) = self.publisher.lock() {
                publisher.fault(subscription);
            }
        }
    }

    fn shutdown(mut self) {
        // Publication is detached before the reader enters close mode; the
        // lifecycle helper then abandons parser work and keeps raw-draining
        // through child termination and reap. Keep the renderer channel alive
        // until the reader has reported Exit; close() removes the session from
        // the runtime before shutdown begins, so this is its final message.
        if let Ok(mut publisher) = self.publisher.lock() {
            publisher.close();
        }
        if let Some(reader) = self.reader.take() {
            #[cfg(unix)]
            {
                let _ = buzz_terminal::lifecycle::shutdown_draining(&mut self.child, reader);
            }
            #[cfg(not(unix))]
            {
                reader.begin_closing();
                let _ = self.child.kill();
                let _ = self.child.wait();
                reader.stop();
                reader.join();
            }
        }
        if let Ok(mut channel) = self.channel.lock() {
            *channel = None;
        }
    }
}

#[derive(Default)]
pub(crate) struct TerminalSessions(Mutex<Vec<Session>>);

impl TerminalSessions {
    fn with_session<T>(&self, id: &str, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
        let id = Uuid::parse_str(id).map_err(|_| "invalid terminal session id".to_string())?;
        let mut sessions = self.0.lock().map_err(|e| e.to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == id)
            .ok_or_else(|| "terminal session not found".to_string())?;
        f(session)
    }

    pub(crate) fn shutdown_all(&self) {
        let sessions = self
            .0
            .lock()
            .map(|mut sessions| std::mem::take(&mut *sessions))
            .unwrap_or_default();
        for session in sessions {
            session.shutdown();
        }
    }
}

fn size(columns: u16, rows: u16) -> Result<Size> {
    if columns == 0 || rows == 0 {
        return Err("terminal dimensions must be non-zero".to_string());
    }
    Ok(Size {
        columns: usize::from(columns),
        screen_lines: usize::from(rows),
        ..Size::default()
    })
}

fn pty_size(columns: u16, rows: u16, pixel_width: u16, pixel_height: u16) -> PtySize {
    PtySize {
        rows,
        cols: columns,
        pixel_width,
        pixel_height,
    }
}

fn offer_capture(
    publisher: &Mutex<FramePublisher>,
    frame: Frame,
    snapshot: impl FnOnce() -> Frame,
) -> Option<Publication> {
    let offered = publisher.lock().ok()?.offer(frame);
    match offered {
        Ok(publication) => publication,
        Err(OfferError::PendingFrameMustBeSnapshot) => {
            publisher.lock().ok()?.offer(snapshot()).ok().flatten()
        }
    }
}

#[tauri::command]
pub(crate) fn terminal_attach(
    request: AttachRequest,
    on_frame: Channel<TerminalMessage>,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<AttachResponse> {
    let terminal_size = size(request.columns, request.rows)?;
    let mut sessions = state.0.lock().map_err(|e| e.to_string())?;

    if let Some(existing_id) = request.session_id.as_deref() {
        let existing_id =
            Uuid::parse_str(existing_id).map_err(|_| "invalid terminal session id".to_string())?;
        let session = sessions
            .iter_mut()
            .find(|session| session.id == existing_id)
            .ok_or_else(|| "terminal session not found".to_string())?;
        let subscription = SubscriptionId::new();
        let mut encoder = buzz_terminal::damage::Encoder::new();
        let snapshot = session.terminal.snapshot(&mut encoder);
        let viewport = snapshot.viewport;
        *session.channel.lock().map_err(|e| e.to_string())? = Some(on_frame);
        let bootstrap = session
            .publisher
            .lock()
            .map_err(|e| e.to_string())?
            .attach(subscription, snapshot)
            .map_err(|_| "terminal snapshot rejected".to_string())?;
        session.publish(bootstrap);
        return Ok(AttachResponse {
            session_id: existing_id.to_string(),
            subscription_id: subscription.to_string(),
            viewport: viewport.into(),
        });
    }

    if sessions.len() >= MAX_LIVE_SESSIONS {
        return Err("too many live terminal sessions".to_string());
    }

    let id = Uuid::new_v4();
    let pair = native_pty_system()
        .openpty(pty_size(
            request.columns,
            request.rows,
            request.pixel_width,
            request.pixel_height,
        ))
        .map_err(|e| e.to_string())?;
    let shell = {
        #[cfg(unix)]
        {
            buzz_terminal::shell::resolve_shell(std::env::var("SHELL").ok().as_deref())
        }
        #[cfg(windows)]
        {
            buzz_terminal::shell::resolve_shell(std::env::var("ComSpec").ok().as_deref())
        }
    };
    let mut command = CommandBuilder::new_default_prog();
    buzz_terminal::env_fence::fence_env(
        &mut command,
        &buzz_terminal::path::user_shell_path(),
        &shell,
    );
    let context = GuiContext {
        channel_id: request.channel_id,
        channel_name: request.channel_name,
        thread_id: request.thread_id,
        npub: request.npub,
        relay_url: request.relay_url,
        session_id: id.to_string(),
    };
    for (key, value) in context_vars(&context) {
        command.env(key, value);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| e.to_string())?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = Arc::new(Mutex::new(
        pair.master.take_writer().map_err(|e| e.to_string())?,
    ));
    let (terminal, actions) = Terminal::new(terminal_size, Fences::default());
    let terminal = Arc::new(SharedTerminal::new(terminal));
    let publisher = Arc::new(Mutex::new(FramePublisher::new(terminal.lock().viewport())));
    let channel = Arc::new(Mutex::new(Some(on_frame)));
    let current_pty_size = Arc::new(Mutex::new(pty_size(
        request.columns,
        request.rows,
        request.pixel_width,
        request.pixel_height,
    )));

    // Emulator replies (DSR/size) must go back through the PTY even when no
    // renderer is attached. The action thread ends when the terminal drops.
    let action_writer = Arc::clone(&writer);
    let action_size = Arc::clone(&current_pty_size);
    let action_channel = Arc::clone(&channel);
    std::thread::spawn(move || {
        while let Ok(action) = actions.recv() {
            let presentation = match &action {
                buzz_terminal::Action::Title(title) => Some(TerminalMessage::Title(title.clone())),
                buzz_terminal::Action::ResetTitle => Some(TerminalMessage::ResetTitle),
                buzz_terminal::Action::Bell => Some(TerminalMessage::Bell),
                _ => None,
            };
            if let Some(message) = presentation {
                let _ = action_channel
                    .lock()
                    .ok()
                    .and_then(|channel| channel.as_ref()?.send(message).ok());
                continue;
            }
            let size = action_size.lock().map(|size| *size).unwrap_or_default();
            if let Some(reply) = buzz_terminal::listener::reply(
                action,
                size.cols,
                size.rows,
                size.pixel_width,
                size.pixel_height,
            ) {
                if action_writer.lock().map_or(true, |mut writer| {
                    writer.write_all(reply.as_bytes()).is_err()
                }) {
                    break;
                }
            }
        }
    });

    let reader_terminal = Arc::clone(&terminal);
    let reader_publisher = Arc::clone(&publisher);
    let reader_channel = Arc::clone(&channel);
    let reader_stopping = Arc::new(AtomicBool::new(false));
    let thread_stopping = Arc::clone(&reader_stopping);
    let reader_handle = std::thread::spawn(move || {
        let mut buffer = [0u8; 16 * 1024];
        let mut encoder = buzz_terminal::damage::Encoder::new();
        let mut snapshot_encoder = buzz_terminal::damage::Encoder::new();
        loop {
            let count = match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            if thread_stopping.load(Ordering::Acquire) {
                break;
            }
            if reader_terminal.is_closing() {
                continue;
            }
            let _ = feed_and_drain(&reader_terminal, &buffer[..count]);
            if reader_terminal.is_closing() {
                continue;
            }
            let needs_snapshot = reader_publisher
                .lock()
                .map(|publisher| publisher.requires_snapshot())
                .unwrap_or(false);
            let frame = if needs_snapshot {
                reader_terminal.snapshot(&mut snapshot_encoder)
            } else {
                reader_terminal.render(&mut encoder)
            };
            if frame.is_empty() {
                continue;
            }
            let publication = offer_capture(&reader_publisher, frame, || {
                reader_terminal.snapshot(&mut snapshot_encoder)
            });
            if let Some(publication) = publication {
                let subscription = publication.subscription_id;
                let result = wire_publication(publication).and_then(|mut message| {
                    let (bracketed_paste, focus_reporting) = reader_terminal.input_modes();
                    message.bracketed_paste = bracketed_paste;
                    message.focus_reporting = focus_reporting;
                    reader_channel
                        .lock()
                        .map_err(|e| e.to_string())?
                        .as_ref()
                        .ok_or_else(|| "detached".to_string())?
                        .send(TerminalMessage::Frame(message))
                        .map_err(|e| e.to_string())
                });
                if result.is_err() {
                    if let Ok(mut publisher) = reader_publisher.lock() {
                        publisher.fault(subscription);
                    }
                }
            }
        }
        let _ = reader_channel
            .lock()
            .ok()
            .and_then(|channel| channel.as_ref()?.send(TerminalMessage::Exit).ok());
    });

    let subscription = SubscriptionId::new();
    let mut snapshot_encoder = buzz_terminal::damage::Encoder::new();
    let viewport = terminal.lock().viewport();
    let bootstrap = publisher
        .lock()
        .map_err(|e| e.to_string())?
        .attach(subscription, terminal.snapshot(&mut snapshot_encoder))
        .map_err(|_| "terminal snapshot rejected".to_string())?;
    let session = Session {
        id,
        terminal: Arc::clone(&terminal),
        master: Some(pair.master),
        writer,
        pty_size: current_pty_size,
        child,
        reader: Some(Box::new(ReaderThread {
            handle: Some(reader_handle),
            terminal: Arc::clone(&terminal),
            stopping: reader_stopping,
        })),
        publisher,
        channel,
    };
    session.publish(bootstrap);
    sessions.push(session);
    Ok(AttachResponse {
        session_id: id.to_string(),
        subscription_id: subscription.to_string(),
        viewport: viewport.into(),
    })
}

#[tauri::command]
pub(crate) fn terminal_detach(
    session_id: String,
    subscription_id: String,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<()> {
    state.with_session(&session_id, |session| {
        let id = SubscriptionId::parse(&subscription_id)?;
        let detached = session
            .publisher
            .lock()
            .map_err(|e| e.to_string())?
            .fault(id);
        if detached {
            *session.channel.lock().map_err(|e| e.to_string())? = None;
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn terminal_close(
    session_id: String,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<()> {
    let id = Uuid::parse_str(&session_id).map_err(|_| "invalid terminal session id".to_string())?;
    let session = {
        let mut sessions = state.0.lock().map_err(|e| e.to_string())?;
        let index = sessions
            .iter()
            .position(|session| session.id == id)
            .ok_or_else(|| "terminal session not found".to_string())?;
        sessions.remove(index)
    };
    session.shutdown();
    Ok(())
}

#[tauri::command]
pub(crate) fn terminal_input(
    session_id: String,
    data: String,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<()> {
    if data.len() > MAX_INPUT_BYTES {
        return Err("terminal input exceeds 1 MiB".to_string());
    }
    state.with_session(&session_id, |session| {
        session
            .writer
            .lock()
            .map_err(|e| e.to_string())?
            .write_all(data.as_bytes())
            .map_err(|e| e.to_string())?;
        // Typing returns you to the live edge, as every terminal does. It has
        // to be explicit: the grid pins a scrolled-back viewport and lets new
        // output pile up above it, so the echo of this keystroke would land on
        // a screen the user cannot see. Doing it here rather than in the
        // renderer covers pasted text and encoded keys for free, and costs a
        // comparison -- not a repaint -- when nothing was scrolled.
        if session.terminal.scroll_to_bottom() {
            publish_viewport(session)?;
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn terminal_resize(
    session_id: String,
    columns: u16,
    rows: u16,
    pixel_width: u16,
    pixel_height: u16,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<WireViewport> {
    let terminal_size = size(columns, rows)?;
    state.with_session(&session_id, |session| {
        let new_pty_size = pty_size(columns, rows, pixel_width, pixel_height);
        session
            .master
            .as_ref()
            .ok_or_else(|| "terminal is closing".to_string())?
            .resize(new_pty_size)
            .map_err(|e| e.to_string())?;
        *session.pty_size.lock().map_err(|e| e.to_string())? = new_pty_size;
        let viewport = session.terminal.resize(terminal_size);
        let publication = {
            let mut publisher = session.publisher.lock().map_err(|e| e.to_string())?;
            publisher.resize_applied(viewport);
            let mut encoder = buzz_terminal::damage::Encoder::new();
            publisher
                .offer(session.terminal.snapshot(&mut encoder))
                .map_err(|_| "terminal resize snapshot rejected".to_string())?
        };
        if let Some(publication) = publication {
            session.publish(publication);
        }
        Ok(viewport.into())
    })
}

/// Republish the viewport after the *viewport itself* moved without the grid
/// changing.
///
/// A snapshot, not a damage capture, and that is load-bearing twice over.
/// `capture_all` deliberately leaves damage alone (`damage.rs`), so the
/// full-damage flag that `scroll_display` just set survives for the reader
/// thread's next `render()` to consume -- which is what clears *its* dedup
/// hashes. Take the damage here instead and those hashes would go on
/// describing the rows the renderer had before the scroll, free to suppress a
/// row that genuinely changed underneath it.
///
/// The encoder is fresh and local because its only job is to encode one frame
/// that is `full` by construction; sharing the reader's would be sharing dedup
/// state across two coordinate systems.
fn publish_viewport(session: &Session) -> Result<()> {
    let mut encoder = buzz_terminal::damage::Encoder::new();
    let publication = session
        .publisher
        .lock()
        .map_err(|e| e.to_string())?
        .offer(session.terminal.snapshot(&mut encoder))
        .map_err(|_| "terminal scroll snapshot rejected".to_string())?;
    if let Some(publication) = publication {
        session.publish(publication);
    }
    Ok(())
}

/// Scroll the viewport through scrollback.
///
/// `lines` is a DOM wheel delta in cells; see [`scroll_by_dom_lines`] for the
/// sign convention and why it is written against `deltaY`.
#[tauri::command]
pub(crate) fn terminal_scroll(
    session_id: String,
    lines: DomLines,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<()> {
    state.with_session(&session_id, |session| {
        // Momentum keeps delivering events for a second or so after the
        // fingers lift. Once history runs out the engine clamps and reports
        // that nothing moved, so the tail costs a lock and a compare rather
        // than a full-grid copy and a frame each.
        if !scroll_by_dom_lines(&session.terminal, lines) {
            return Ok(());
        }
        publish_viewport(session)
    })
}

#[tauri::command]
pub(crate) fn terminal_ack(
    session_id: String,
    subscription_id: String,
    sequence: u64,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<()> {
    state.with_session(&session_id, |session| {
        let id = SubscriptionId::parse(&subscription_id)?;
        let publication = session
            .publisher
            .lock()
            .map_err(|e| e.to_string())?
            .acknowledge(id, sequence);
        if let Some(publication) = publication {
            session.publish(publication);
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn terminal_viewport_ready(
    session_id: String,
    subscription_id: String,
    viewport: WireViewport,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<()> {
    state.with_session(&session_id, |session| {
        let id = SubscriptionId::parse(&subscription_id)?;
        let publication = session
            .publisher
            .lock()
            .map_err(|e| e.to_string())?
            .viewport_ready(id, viewport.into());
        if let Some(publication) = publication {
            session.publish(publication);
        }
        Ok(())
    })
}

#[tauri::command]
pub(crate) fn terminal_focus(
    session_id: String,
    focused: bool,
    state: tauri::State<'_, TerminalSessions>,
) -> Result<()> {
    state.with_session(&session_id, |session| {
        let (_, enabled) = session.terminal.input_modes();
        if enabled {
            session
                .writer
                .lock()
                .map_err(|e| e.to_string())?
                .write_all(if focused { b"\x1b[I" } else { b"\x1b[O" })
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_terminal::damage::{CursorFrame, RowFrame, Span};

    fn publication(spans: Vec<Span>) -> Publication {
        Publication {
            subscription_id: SubscriptionId::new(),
            sequence: 7,
            frame: buzz_terminal::damage::Frame {
                rows: vec![RowFrame {
                    line: 3,
                    wrapped: true,
                    spans,
                }],
                cursor: CursorFrame {
                    line: 1,
                    column: 2,
                    visible: true,
                },
                cursor_changed: true,
                full: true,
                viewport: Viewport {
                    generation: 4,
                    columns: 80,
                    screen_lines: 24,
                },
            },
        }
    }

    fn style() -> Style {
        Style {
            fg: 1,
            bg: 2,
            flags: 3,
        }
    }

    fn marker_frame(marker: usize, full: bool) -> Frame {
        Frame {
            rows: vec![RowFrame {
                line: marker,
                wrapped: false,
                spans: Vec::new(),
            }],
            cursor: CursorFrame {
                line: 0,
                column: 0,
                visible: true,
            },
            cursor_changed: true,
            full,
            viewport: Viewport {
                generation: 0,
                columns: 80,
                screen_lines: 24,
            },
        }
    }

    fn assert_post_snapshot_capture_survives_attach(mut publisher: FramePublisher) {
        assert!(!publisher.requires_snapshot());
        let bootstrap = marker_frame(1, true);
        let post_snapshot_incremental = marker_frame(42, false);
        let subscription = SubscriptionId::new();
        publisher.attach(subscription, bootstrap).unwrap();
        let publisher = Mutex::new(publisher);

        assert_eq!(
            offer_capture(&publisher, post_snapshot_incremental, || marker_frame(
                42, true
            )),
            None
        );

        let successor = publisher
            .lock()
            .unwrap()
            .acknowledge(subscription, 1)
            .expect("post-snapshot PTY output was lost");
        assert_eq!(successor.frame.rows[0].line, 42);
        assert!(successor.frame.full);
    }

    #[test]
    fn reader_pumps_a_deferred_tail_without_an_external_event() {
        let (terminal, _actions) = Terminal::new(Size::default(), Fences::ALL);
        let terminal = SharedTerminal::new(terminal);
        let payload = "\u{1b}c".repeat(2_102_714);

        assert!(
            feed_and_drain(&terminal, payload.as_bytes()),
            "fixture must defer parser work before the runtime pumps it"
        );

        let terminal = terminal.lock();
        assert_eq!(terminal.pending_bytes(), 0);
        assert_eq!(terminal.stats().completed_units, 2_102_714);
    }

    #[test]
    fn initial_attach_retains_output_captured_after_its_bootstrap_snapshot() {
        let viewport = marker_frame(0, true).viewport;
        let publisher = FramePublisher::new(viewport);
        assert_post_snapshot_capture_survives_attach(publisher);
    }

    #[test]
    fn reattach_retains_output_captured_after_its_bootstrap_snapshot() {
        let viewport = marker_frame(0, true).viewport;
        let mut publisher = FramePublisher::new(viewport);
        let old = SubscriptionId::new();
        publisher.attach(old, marker_frame(0, true)).unwrap();
        assert!(publisher.acknowledge(old, 1).is_none());
        assert_post_snapshot_capture_survives_attach(publisher);
    }

    #[test]
    fn mapper_preserves_soft_wrap_metadata() {
        let message = wire_publication(publication(Vec::new())).unwrap();
        assert!(message.rows[0].wrapped);
    }

    #[test]
    fn mapper_expands_ascii_runs_without_unicode_classification() {
        let message = wire_publication(publication(vec![Span {
            column: 4,
            text: "abc".into(),
            width: 1,
            cluster_count: 3,
            style: style(),
        }]))
        .unwrap();
        let clusters = &message.rows[0].spans[0].clusters;
        assert_eq!(
            clusters
                .iter()
                .map(|cluster| (cluster.column, cluster.text.as_str()))
                .collect::<Vec<_>>(),
            vec![(4, "a"), (5, "b"), (6, "c")]
        );
    }

    #[test]
    fn mapper_keeps_a_multi_char_cluster_atomic() {
        let message = wire_publication(publication(vec![Span {
            column: 9,
            text: "1\u{fe0f}\u{20e3}".into(),
            width: 2,
            cluster_count: 1,
            style: style(),
        }]))
        .unwrap();
        let clusters = &message.rows[0].spans[0].clusters;
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].column, 9);
        assert_eq!(clusters[0].text, "1\u{fe0f}\u{20e3}");
        assert_eq!(clusters[0].width, 2);
    }

    #[test]
    fn mapper_rejects_an_inconsistent_engine_span() {
        let result = wire_publication(publication(vec![Span {
            column: 0,
            text: "ab".into(),
            width: 1,
            cluster_count: 3,
            style: style(),
        }]));
        assert!(result.is_err());
    }

    #[test]
    fn dimensions_reject_zero_and_preserve_scrollback_default() {
        assert!(size(0, 24).is_err());
        assert!(size(80, 0).is_err());
        let size = size(100, 40).unwrap();
        assert_eq!(
            (size.columns, size.screen_lines, size.scrollback),
            (100, 40, 10_000)
        );
    }
}
