//! Narrow MCP `2026-07-28` JSON-RPC stdio client.
//!
//! Speaks exactly `server/discover`, `tools/list`, and `tools/call` over
//! newline-delimited JSON-RPC on a child process's stdin/stdout, per
//! an empty environment except `LANG`, `TMPDIR`, the
//! plugin id, and the contract version; its own process group; a 1 MiB
//! stdout frame cap; an 8 KiB stderr tail to the app log only; per-request
//! `_meta`; and duplicate/unknown response ids treated as a protocol
//! violation that ends the session.
//!
//! Process-group containment (`SIGTERM`/`SIGKILL` to the whole group) is
//! implemented for Unix only; on other platforms it is a documented no-op,
//! since this transport has no non-Unix equivalent yet.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

#[path = "process_owner.rs"]
mod process_owner;
use process_owner::{GroupSignal, ProcessOwner};

use crate::plugin_host::types::{Admitted, PluginError, ResolveKind};

/// MCP protocol revision this client speaks; the only one it accepts.
const PROTOCOL_VERSION: &str = "2026-07-28";
/// Newline-delimited JSON-RPC frame cap in both directions.
const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Bytes of stderr retained per session for the app log.
const STDERR_TAIL_BYTES: usize = 8 * 1024;
/// Fixed chunk size for stderr reads; bounds allocation regardless of
/// whether the child ever writes a newline.
const STDERR_CHUNK_BYTES: usize = 4096;
/// Longest single stderr line excerpt written to the app log. Bytes beyond
/// this are still folded into the retained tail, just not logged per-line.
const STDERR_LOG_LINE_CAP: usize = 200;
/// Bound on remembered completed response ids. Ids are issued strictly
/// increasing, one call outstanding at a time, so only a small recent window
/// is ever needed to catch a duplicate; this keeps a long-lived session's
/// memory bounded instead of growing with every call ever made.
const COMPLETED_ID_RETENTION: usize = 1024;
/// Sleep between `try_wait` polls while reaping a signaled process.
const REAP_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CLIENT_NAME: &str = "buzz-desktop";
const REQUIRED_TOOL_NAME: &str = "browser.resolve";

/// One live plugin process speaking the narrow MCP stdio contract.
pub struct Client {
    stdin: Mutex<Option<ChildStdin>>,
    pending: Arc<Mutex<PendingState>>,
    process: Arc<ProcessOwner>,
    #[cfg_attr(not(test), allow(dead_code))]
    stderr_tail: Arc<Mutex<VecDeque<u8>>>,
}

/// Bounded record of ids already delivered. `insert` returns `true` the first
/// time an id is recorded and `false` for a duplicate; retention is capped so
/// memory does not grow with the number of calls made over a session's
/// lifetime.
#[derive(Default)]
struct CompletedIds {
    seen: HashSet<u64>,
    order: VecDeque<u64>,
}

impl CompletedIds {
    fn insert(&mut self, id: u64) -> bool {
        if !self.seen.insert(id) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > COMPLETED_ID_RETENTION {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        true
    }
}

/// What a call's waiter is resolved with. Kept as a Rust-level enum, not a
/// JSON sentinel field inside a `Value`, so a plugin's own `result` payload
/// can never be misread as an internal signal — `dispatch_line` is the only
/// place that ever constructs `Delivery::Result`/`Delivery::Error`, both from
/// bytes the plugin actually sent, and `Delivery::Superseded` never crosses
/// the wire at all.
enum Delivery {
    Result(Value),
    Error(Value),
    Superseded,
}

struct PendingState {
    next_id: u64,
    completed: CompletedIds,
    outstanding: std::collections::HashMap<u64, oneshot::Sender<Delivery>>,
    /// The most recent `tools/call` id; a new call cancels the previous
    /// waiter (its sender is dropped, so its `await` resolves to an error)
    /// rather than leaving two calls racing over one process's stdout.
    current_call: Option<u64>,
    fatal: Option<PluginError>,
}

impl Client {
    /// Spawns `executable` with an empty environment except `LANG`,
    /// `TMPDIR`, `plugin_id`, and `contract_version`, in its own process
    /// group, and starts the background stdout/stderr readers.
    pub fn spawn(
        executable: &Path,
        plugin_id: &str,
        contract_version: &str,
    ) -> Result<Arc<Client>, PluginError> {
        let mut command = Command::new(executable);
        command.env_clear();
        if let Ok(lang) = std::env::var("LANG") {
            command.env("LANG", lang);
        }
        if let Ok(tmpdir) = std::env::var("TMPDIR") {
            command.env("TMPDIR", tmpdir);
        }
        command.env("BUZZ_PLUGIN_ID", plugin_id);
        command.env("BUZZ_PLUGIN_CONTRACT_VERSION", contract_version);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);

        let mut child = command
            .spawn()
            .map_err(|error| PluginError::PluginProtocol(format!("spawn plugin: {error}")))?;
        #[cfg(unix)]
        let pgid = child.id();
        #[cfg(not(unix))]
        let pgid: Option<u32> = None;
        let stdin = child.stdin.take().ok_or(PluginError::PluginUnavailable)?;
        let stdout = child.stdout.take().ok_or(PluginError::PluginUnavailable)?;
        let stderr = child.stderr.take().ok_or(PluginError::PluginUnavailable)?;

        let pending = Arc::new(Mutex::new(PendingState {
            next_id: 1,
            completed: CompletedIds::default(),
            outstanding: std::collections::HashMap::new(),
            current_call: None,
            fatal: None,
        }));

        let stderr_tail = Arc::new(Mutex::new(VecDeque::with_capacity(STDERR_TAIL_BYTES)));

        let client = Arc::new(Client {
            stdin: Mutex::new(Some(stdin)),
            pending: Arc::clone(&pending),
            process: Arc::new(ProcessOwner::new(child, pgid)),
            stderr_tail: Arc::clone(&stderr_tail),
        });

        tokio::spawn(run_stdout_reader(
            stdout,
            Arc::clone(&pending),
            Arc::clone(&client.process),
        ));
        tokio::spawn(run_stderr_tail(stderr, plugin_id.to_string(), stderr_tail));

        Ok(client)
    }

    /// Snapshot of the retained stderr tail, for tests that need to assert
    /// the cap directly rather than only observing session responsiveness.
    #[cfg(test)]
    pub(crate) async fn stderr_tail_snapshot(&self) -> Vec<u8> {
        self.stderr_tail.lock().await.iter().copied().collect()
    }

    /// Sends `server/discover` and validates that the plugin supports this
    /// protocol revision and advertises tool capability.
    pub async fn discover(&self, deadline: Duration) -> Result<(), PluginError> {
        let result = self
            .send(
                "server/discover",
                json!({
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
                        "io.modelcontextprotocol/clientCapabilities": {},
                        "io.modelcontextprotocol/clientInfo": {
                            "name": CLIENT_NAME, "version": env!("CARGO_PKG_VERSION")
                        }
                    }
                }),
                false,
                deadline,
            )
            .await?;
        let supported = result["supportedVersions"]
            .as_array()
            .map(|versions| {
                versions
                    .iter()
                    .any(|version| version.as_str() == Some(PROTOCOL_VERSION))
            })
            .unwrap_or(false);
        let has_tools_capability = result["capabilities"]["tools"].is_object();
        if !supported || !has_tools_capability {
            return Err(PluginError::ContractMismatch(
                "discover result omits the supported protocol version or tools capability".into(),
            ));
        }
        Ok(())
    }

    /// Sends `tools/list` and validates that the plugin advertises exactly
    /// one tool, named `browser.resolve`.
    pub async fn tools_list(&self, deadline: Duration) -> Result<(), PluginError> {
        let result = self
            .send(
                "tools/list",
                json!({
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
                        "io.modelcontextprotocol/clientCapabilities": {}
                    }
                }),
                false,
                deadline,
            )
            .await?;
        let tools = result["tools"].as_array().ok_or_else(|| {
            PluginError::ContractMismatch("tools/list result has no tools array".into())
        })?;
        if tools.len() != 1 || tools[0]["name"].as_str() != Some(REQUIRED_TOOL_NAME) {
            return Err(PluginError::ContractMismatch(format!(
                "tools/list must advertise exactly one {REQUIRED_TOOL_NAME:?} tool"
            )));
        }
        Ok(())
    }

    /// Sends `tools/call` for `browser.resolve` and interprets its result.
    ///
    /// A plugin refusal (`outcome: "rejected"`) and a host navigation
    /// refusal of a `resolved` URL both surface as
    /// [`PluginError::NavigationDenied`] — the caller never distinguishes
    /// them, matching the design's "discard the plugin's reason" rule, and
    /// neither one invalidates the session: the process stays usable for the
    /// next call. Any other shape violation (wrong `resultType`, missing
    /// `outcome`, a `resolved` answer with no `url`, or an unrecognized
    /// `outcome` value) is a protocol violation and invalidates the session,
    /// the same as a wire-framing violation.
    #[allow(clippy::too_many_arguments)]
    pub async fn resolve(
        &self,
        plugin_id: &str,
        contribution_id: &str,
        session_id: &str,
        generation: u64,
        contract_version: &str,
        kind: ResolveKind,
        input: Option<&str>,
        deadline: Duration,
    ) -> Result<Admitted, PluginError> {
        let kind_str = match kind {
            ResolveKind::Home => "home",
            ResolveKind::Address => "address",
        };
        let mut arguments = json!({
            "context": {
                "contractVersion": contract_version,
                "pluginId": plugin_id,
                "contributionId": contribution_id,
                "sessionId": session_id,
                "generation": generation,
            },
            "kind": kind_str,
        });
        if let Some(input) = input {
            arguments["input"] = json!(input);
        }
        let params = json!({
            "name": REQUIRED_TOOL_NAME,
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities": {}
            },
            "arguments": arguments,
        });
        let result = self.send("tools/call", params, true, deadline).await?;

        if result["resultType"].as_str() != Some("complete") {
            let error =
                PluginError::PluginProtocol("tools/call result is not resultType=complete".into());
            self.invalidate(error.clone()).await;
            return Err(error);
        }
        let structured = &result["structuredContent"];
        let Some(outcome) = structured["outcome"].as_str() else {
            let error = PluginError::PluginProtocol("tool result has no outcome".into());
            self.invalidate(error.clone()).await;
            return Err(error);
        };
        match outcome {
            "resolved" => {
                let Some(url) = structured["url"].as_str() else {
                    let error = PluginError::PluginProtocol("resolved result has no url".into());
                    self.invalidate(error.clone()).await;
                    return Err(error);
                };
                let url = url.to_string();
                let title = structured["title"].as_str().map(str::to_string);
                match crate::plugin_host::navigation::parse_admitted(&url) {
                    Some(_) => Ok(Admitted { url, title }),
                    None => Err(PluginError::NavigationDenied { address: url }),
                }
            }
            "rejected" => Err(PluginError::NavigationDenied {
                address: input.unwrap_or_default().to_string(),
            }),
            other => {
                let error = PluginError::PluginProtocol(format!("unknown tool outcome {other:?}"));
                self.invalidate(error.clone()).await;
                Err(error)
            }
        }
    }

    /// Whether group cleanup and leader reaping are both complete.
    pub fn is_terminated(&self) -> bool {
        self.process.is_terminated()
    }

    #[cfg(test)]
    pub(super) fn fail_termination_for_test(&self, fail: bool) {
        self.process.fail_signals_for_test(fail);
    }

    async fn reap_async(&self, deadline: std::time::Instant) -> Result<(), PluginError> {
        loop {
            if let Some(result) = self.process.try_reap() {
                return result;
            }
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                return Err(protocol_error("plugin process did not exit after SIGKILL"));
            };
            tokio::time::sleep(REAP_POLL_INTERVAL.min(remaining)).await;
        }
    }

    fn reap_blocking(&self, deadline: std::time::Instant) -> Result<(), PluginError> {
        loop {
            if let Some(result) = self.process.try_reap() {
                return result;
            }
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                return Err(protocol_error("plugin process did not exit after SIGKILL"));
            };
            std::thread::sleep(REAP_POLL_INTERVAL.min(remaining));
        }
    }

    /// Closes stdin, terminates the whole group, then reaps its leader.
    /// Cancellation or signal failure retains ownership for another cleanup.
    pub async fn shutdown(&self, grace: Duration) -> Result<(), PluginError> {
        if self.is_terminated() {
            return Ok(());
        }
        self.stdin.lock().await.take();
        let termination = self.process.signal(GroupSignal::Term);
        tokio::time::sleep(grace).await;
        self.process.signal(GroupSignal::Kill)?;
        self.reap_async(std::time::Instant::now() + grace).await?;
        termination
    }

    /// Marks every current and future call fatal, without touching the
    /// process group. Split out from [`Client::invalidate`] so `send()` can
    /// poison the session while still holding the stdin lock (closing the
    /// window where a writer already queued behind it could interleave bytes
    /// with a torn frame), and send the kill signal only after that lock is
    /// released.
    async fn poison(&self, error: PluginError) {
        fail_all(&self.pending, error).await;
    }

    async fn kill_process_group(&self) {
        if let Err(error) = self.process.signal(GroupSignal::Kill) {
            eprintln!("buzz-desktop: {error:?}");
        }
    }

    /// Marks the session fatally invalid and kills its process group. Used
    /// both by the background stdout reader (wire-framing violations) and by
    /// [`Client::resolve`] (result-shape violations) so every protocol
    /// violation ends the session the same way.
    async fn invalidate(&self, error: PluginError) {
        self.poison(error).await;
        self.kill_process_group().await;
    }

    async fn send(
        &self,
        method: &str,
        params: Value,
        is_call: bool,
        deadline: Duration,
    ) -> Result<Value, PluginError> {
        let (id, rx) = {
            let mut pending = self.pending.lock().await;
            if let Some(fatal) = pending.fatal.clone() {
                return Err(fatal);
            }
            let id = pending.next_id;
            pending.next_id += 1;
            if is_call {
                if let Some(previous_id) = pending.current_call.replace(id) {
                    // Deliver a distinguishable marker rather than just
                    // dropping the sender: a plain drop resolves the
                    // superseded call's `rx.await` to a cancellation, which
                    // this function cannot tell apart from a real session
                    // death and would report as `PluginUnavailable` — wrongly
                    // implying the plugin process itself failed. Reusing
                    // `PluginError::StaleGeneration` here (below) lets the
                    // existing host-side handling
                    // (`commands.rs::browser_error_code`, which already maps
                    // it to no user-facing event) suppress it the same way
                    // it already suppresses an actual generation bump,
                    // matching the contract's "the earlier one is cancelled
                    // and its result discarded" — with no host-side change
                    // needed.
                    if let Some(previous_sender) = pending.outstanding.remove(&previous_id) {
                        let _ = previous_sender.send(Delivery::Superseded);
                    }
                }
            }
            let (tx, rx) = oneshot::channel();
            pending.outstanding.insert(id, tx);
            (id, rx)
        };

        let frame = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut line = serde_json::to_vec(&frame)
            .map_err(|error| PluginError::PluginProtocol(format!("encode request: {error}")))?;
        line.push(b'\n');
        if line.len() > MAX_FRAME_BYTES {
            self.remove_pending(id).await;
            let error = PluginError::PluginProtocol(format!(
                "outgoing frame of {} bytes exceeds the {MAX_FRAME_BYTES}-byte cap",
                line.len()
            ));
            // Per contract, a line over the cap ends the session rather than
            // merely failing this call: nothing was written, but a caller
            // that produced one oversized frame is not a transport this
            // client should keep trusting with further frames.
            self.invalidate(error.clone()).await;
            return Err(error);
        }

        // A single absolute instant bounds the stdin lock wait, the write,
        // the flush, and the response wait together, not just the wait: a
        // child that never reads its stdin must not be able to block this
        // call past `deadline` while sitting outside a timed region. Using
        // one `Instant` (rather than re-arming a fresh `Duration` per phase)
        // means the response-wait phase does not get a full fresh `deadline`
        // after time has already been spent waiting for the stdin lock.
        let deadline_instant = tokio::time::Instant::now() + deadline;

        // The write phase races the write+flush against the deadline while
        // still holding the stdin lock, and — on the failure paths — poisons
        // the session (`self.poison`) before that lock is released. This
        // closes a race where a writer already queued behind this one could
        // otherwise acquire the stream and write more bytes before this call
        // got a chance to mark the session fatal, interleaving with a torn
        // frame. Symmetrically, once the lock is held, `fatal` is rechecked
        // here too: an earlier concurrent call may have poisoned the stream
        // while this one was waiting for the lock, in which case this call
        // must not write into an already-poisoned stream either.
        enum WriteOutcome {
            Written,
            NoStdin,
            AlreadyFatal(PluginError),
            Failed(PluginError),
            WaitingTimedOut,
        }
        let write_outcome = async {
            let Ok(mut stdin_guard) =
                tokio::time::timeout_at(deadline_instant, self.stdin.lock()).await
            else {
                return WriteOutcome::WaitingTimedOut;
            };
            if tokio::time::Instant::now() >= deadline_instant {
                return WriteOutcome::WaitingTimedOut;
            }
            if let Some(fatal) = self.pending.lock().await.fatal.clone() {
                return WriteOutcome::AlreadyFatal(fatal);
            }
            // Cancellation drops this handle before the guard, leaving shared
            // stdin absent. Only a complete frame may restore the stream.
            let Some(mut stdin) = stdin_guard.take() else {
                return WriteOutcome::NoStdin;
            };
            let write_and_flush = async {
                stdin.write_all(&line).await?;
                stdin.flush().await
            };
            let outcome = tokio::select! {
                result = write_and_flush => match result {
                    Ok(()) => WriteOutcome::Written,
                    Err(io_error) => {
                        eprintln!("buzz-desktop: plugin stdin write failed: {io_error}");
                        let error = PluginError::PluginProtocol(format!(
                            "plugin stdin write failed: {io_error}"
                        ));
                        self.poison(error.clone()).await;
                        WriteOutcome::Failed(error)
                    }
                },
                () = tokio::time::sleep_until(deadline_instant) => {
                    let error = PluginError::PluginProtocol(
                        "request write timed out mid-write; the stdin stream may be corrupted"
                            .into(),
                    );
                    self.poison(error.clone()).await;
                    WriteOutcome::Failed(error)
                }
            };
            if matches!(outcome, WriteOutcome::Written) {
                *stdin_guard = Some(stdin);
            }
            outcome
        }
        .await;

        match write_outcome {
            WriteOutcome::Written => {}
            WriteOutcome::WaitingTimedOut => {
                self.remove_pending(id).await;
                return Err(PluginError::PluginTimeout);
            }
            WriteOutcome::NoStdin => {
                self.remove_pending(id).await;
                return Err(PluginError::PluginUnavailable);
            }
            WriteOutcome::AlreadyFatal(error) => {
                self.remove_pending(id).await;
                return Err(error);
            }
            WriteOutcome::Failed(error) => {
                self.remove_pending(id).await;
                self.kill_process_group().await;
                return Err(error);
            }
        }

        match tokio::time::timeout_at(deadline_instant, rx).await {
            Ok(Ok(Delivery::Result(value))) => Ok(value),
            Ok(Ok(Delivery::Error(error))) => Err(PluginError::PluginProtocol(format!(
                "plugin returned a JSON-RPC error: {error}"
            ))),
            Ok(Ok(Delivery::Superseded)) => Err(PluginError::StaleGeneration),
            Ok(Err(_canceled)) => {
                self.remove_pending(id).await;
                let pending = self.pending.lock().await;
                Err(pending
                    .fatal
                    .clone()
                    .unwrap_or(PluginError::PluginUnavailable))
            }
            Err(_elapsed) => {
                self.remove_pending(id).await;
                let pending = self.pending.lock().await;
                if let Some(fatal) = pending.fatal.clone() {
                    return Err(fatal);
                }
                Err(PluginError::PluginTimeout)
            }
        }
    }

    /// Removes a request's waiter and, if it was the tracked in-flight call,
    /// clears that tracking too. Called on timeout or cancellation so a
    /// superseded or abandoned request never lingers in `outstanding`.
    async fn remove_pending(&self, id: u64) {
        let mut pending = self.pending.lock().await;
        pending.outstanding.remove(&id);
        if pending.current_call == Some(id) {
            pending.current_call = None;
        }
    }
}

/// Terminates and reaps clients with one shared grace and reap deadline.
/// Every client retains its process owner when any operation fails.
pub fn terminate_clients_blocking(
    clients: &[Arc<Client>],
    grace: Duration,
) -> Result<(), PluginError> {
    let live: Vec<_> = clients
        .iter()
        .filter(|client| !client.is_terminated())
        .collect();
    if live.is_empty() {
        return Ok(());
    }
    let mut first_error = None;
    for client in &live {
        if let Ok(mut stdin) = client.stdin.try_lock() {
            stdin.take();
        }
        if let Err(error) = client.process.signal(GroupSignal::Term) {
            first_error.get_or_insert(error);
        }
    }
    std::thread::sleep(grace);
    let mut signaled = Vec::new();
    for client in &live {
        match client.process.signal(GroupSignal::Kill) {
            Ok(()) => signaled.push(client),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    let deadline = std::time::Instant::now() + grace;
    for client in signaled {
        if let Err(error) = client.reap_blocking(deadline) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn run_stdout_reader(
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<PendingState>>,
    process: Arc<ProcessOwner>,
) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_capped_line(&mut reader).await {
            Ok(None) => {
                fail_all(&pending, PluginError::PluginUnavailable).await;
                return;
            }
            Ok(Some(line)) if line.len() > MAX_FRAME_BYTES => {
                terminate(&pending, &process, protocol_error("frame exceeds 1 MiB")).await;
                return;
            }
            Ok(Some(line)) => {
                if !dispatch_line(&line, &pending, &process).await {
                    return;
                }
            }
            Err(_io_error) => {
                fail_all(&pending, PluginError::PluginUnavailable).await;
                return;
            }
        }
    }
}

/// Reads one newline-delimited frame, capped so a line with no newline
/// cannot grow past the limit before it is rejected.
async fn read_capped_line(
    reader: &mut BufReader<tokio::process::ChildStdout>,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut buf = Vec::new();
    let mut limited = AsyncReadExt::take(reader, (MAX_FRAME_BYTES + 1) as u64);
    let read = limited.read_until(b'\n', &mut buf).await?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(buf))
}

fn protocol_error(reason: &str) -> PluginError {
    PluginError::PluginProtocol(reason.to_string())
}

/// Returns `false` when the session must stop reading (a fatal error was
/// recorded), `true` to keep the reader loop going.
async fn dispatch_line(
    line: &[u8],
    pending: &Arc<Mutex<PendingState>>,
    process: &Arc<ProcessOwner>,
) -> bool {
    let trimmed = trim_newline(line);
    let value: Value = match serde_json::from_slice(trimmed) {
        Ok(value) => value,
        Err(_) => {
            terminate(pending, process, protocol_error("malformed JSON frame")).await;
            return false;
        }
    };
    if !value.is_object() {
        terminate(
            pending,
            process,
            protocol_error("frame is not a JSON object"),
        )
        .await;
        return false;
    }
    if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        terminate(
            pending,
            process,
            protocol_error("frame is missing jsonrpc \"2.0\""),
        )
        .await;
        return false;
    }
    if let Some(method) = value.get("method").and_then(Value::as_str) {
        if method.starts_with("notifications/") {
            // A notification is one-way and carries no `id`, `result`, or
            // `error`; one that does is not a well-formed notification and
            // must not be silently accepted just because its method name
            // matches the ignored prefix.
            if value.get("id").is_some()
                || value.get("result").is_some()
                || value.get("error").is_some()
            {
                terminate(
                    pending,
                    process,
                    protocol_error(&format!("malformed notification: {method}")),
                )
                .await;
                return false;
            }
            return true;
        }
        terminate(
            pending,
            process,
            protocol_error(&format!("unexpected message from plugin: {method}")),
        )
        .await;
        return false;
    }
    // Anything past this point must be a response: exactly one of `result`
    // or `error`, never both and never neither.
    if value.get("result").is_some() == value.get("error").is_some() {
        terminate(
            pending,
            process,
            protocol_error("response must carry exactly one of result or error"),
        )
        .await;
        return false;
    }
    let Some(id) = value.get("id").and_then(Value::as_u64) else {
        terminate(
            pending,
            process,
            protocol_error("response has no integer id"),
        )
        .await;
        return false;
    };

    let mut state = pending.lock().await;
    // Ids are issued strictly increasing starting at 1 with no gaps, so an id
    // was issued if and only if it lies in `[1, next_id)` at this point in
    // time; this needs no separate unbounded "ever issued" set.
    if id == 0 || id >= state.next_id {
        drop(state);
        terminate(
            pending,
            process,
            protocol_error(&format!("response id {id} was never issued")),
        )
        .await;
        return false;
    }
    if !state.completed.insert(id) {
        drop(state);
        terminate(
            pending,
            process,
            protocol_error(&format!("duplicate response id {id}")),
        )
        .await;
        return false;
    }

    let delivery = if let Some(error) = value.get("error") {
        Delivery::Error(error.clone())
    } else {
        Delivery::Result(value.get("result").cloned().unwrap_or(Value::Null))
    };
    if let Some(sender) = state.outstanding.remove(&id) {
        let _ = sender.send(delivery);
    }
    // An id with no outstanding waiter (already timed out and abandoned) is
    // discarded here: it was legitimately issued and is now completing late,
    // which is not the same as an id the host never sent.
    true
}

fn trim_newline(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
    }
    if end > 0 && line[end - 1] == b'\r' {
        end -= 1;
    }
    &line[..end]
}

async fn terminate(
    pending: &Arc<Mutex<PendingState>>,
    process: &Arc<ProcessOwner>,
    error: PluginError,
) {
    fail_all(pending, error).await;
    if let Err(error) = process.signal(GroupSignal::Kill) {
        eprintln!("buzz-desktop: {error:?}");
    }
}

async fn fail_all(pending: &Arc<Mutex<PendingState>>, error: PluginError) {
    let mut state = pending.lock().await;
    state.fatal = Some(error);
    // Drop each sender rather than sending a value: a dropped oneshot::Sender
    // causes the waiting `rx.await` to observe a canceled receive, which
    // `send()` maps to `PluginError::PluginUnavailable` (or the more specific
    // `state.fatal` it re-checks after the timeout path). Sending a
    // placeholder value here would instead look like a real (if malformed)
    // plugin response to the caller.
    state.outstanding.clear();
}

/// Reads stderr in fixed-size chunks — never `read_until`, which would grow
/// its buffer without bound if the child writes newline-free data — and logs
/// each line truncated to `STDERR_LOG_LINE_CAP`, so neither the read nor the
/// log write is sized by however much an adversarial or buggy plugin writes.
/// The retained `tail` buffer still keeps the full last `STDERR_TAIL_BYTES`.
async fn run_stderr_tail(
    stderr: tokio::process::ChildStderr,
    plugin_id: String,
    shared_tail: Arc<Mutex<VecDeque<u8>>>,
) {
    let mut reader = stderr;
    let mut tail: VecDeque<u8> = VecDeque::with_capacity(STDERR_TAIL_BYTES);
    let mut line_buffer: Vec<u8> = Vec::with_capacity(STDERR_LOG_LINE_CAP);
    let mut chunk = [0u8; STDERR_CHUNK_BYTES];
    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for &byte in &chunk[..read] {
            if tail.len() == STDERR_TAIL_BYTES {
                tail.pop_front();
            }
            tail.push_back(byte);

            if byte == b'\n' {
                log_stderr_line(&plugin_id, &line_buffer);
                line_buffer.clear();
            } else if line_buffer.len() < STDERR_LOG_LINE_CAP {
                line_buffer.push(byte);
            }
            // Bytes beyond the log cap for the current line are dropped from
            // the logged excerpt only; `tail` above still retains them.
        }
        // Published once per chunk, not per byte, so a test can assert the
        // retained cap without a lock acquisition per byte.
        {
            let mut shared = shared_tail.lock().await;
            shared.clear();
            shared.extend(tail.iter().copied());
        }
    }
    if !line_buffer.is_empty() {
        log_stderr_line(&plugin_id, &line_buffer);
    }
}

fn log_stderr_line(plugin_id: &str, line: &[u8]) {
    let text = String::from_utf8_lossy(line);
    eprintln!(
        "buzz-desktop: plugin {plugin_id} stderr: {}",
        text.trim_end()
    );
}

#[cfg(test)]
#[path = "mcp_stdio_tests.rs"]
mod tests;
