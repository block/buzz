//! Unix-socket consumer for Hermes Buzz gateway forward frames.
//!
//! One connection carries one length-prefixed JSON frame. Durable occupancy of
//! `dedupe_key` plus admission is the ack criterion (`accepted`), not agent
//! turn completion. The in-memory queue is not durable across a crash.

use std::collections::{HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use buzz_core::kind::{
    KIND_AGENT_OBSERVER_FRAME, KIND_MEMBER_ADDED_NOTIFICATION, KIND_MEMBER_REMOVED_NOTIFICATION,
};
use nostr::Event;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::intake::{EnqueueResult, ForwardWork};
use crate::relay::BuzzEvent;

pub const FORWARD_FRAME_MAX: usize = 8 * 1024 * 1024;
/// Append-only file is unbounded; `load` keeps only the most recent N keys in
/// reload HashSet with bounded parsing memory. Runtime reservations can grow.
/// Keys that remain on
/// disk but fall outside this window are treated as new after restart and may
/// be processed again.
const DEDUPE_MEMORY_CAP: usize = 50_000;
const ACK_TIMEOUT: Duration = Duration::from_secs(10);

pub fn compute_dedupe_key(event_id: &str, direction: &str, channel_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event_id.as_bytes());
    hasher.update(direction.as_bytes());
    hasher.update(channel_id.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn relay_dedupe_key(event: &Event, channel_id: Uuid) -> String {
    compute_dedupe_key(&event.id.to_hex(), "inbound", &channel_id.to_string())
}

#[derive(Debug, Serialize, Deserialize)]
struct ForwardFrame {
    event: Value,
    direction: String,
    /// 예약: wire-contract field. Not used for authorization.
    #[allow(dead_code)]
    chat_type: String,
    channel_id: String,
    dedupe_key: String,
}

#[derive(Clone)]
pub struct DedupeStore {
    path: PathBuf,
    keys: HashSet<String>,
}

fn is_complete_dedupe_line(line: &str) -> bool {
    line.len() == 64 && line.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl DedupeStore {
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        Self::load_with_cap(path, DEDUPE_MEMORY_CAP)
    }

    fn load_with_cap(path: impl AsRef<Path>, cap: usize) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut ordered = VecDeque::new();
        if path.is_file() {
            let file = File::open(&path)?;
            for line in BufReader::new(file).lines() {
                let line = line?;
                let trimmed = line.trim();
                // Skip empty and truncated/partial last lines from a crash mid-write.
                if is_complete_dedupe_line(trimmed) {
                    ordered.push_back(trimmed.to_string());
                    if ordered.len() > cap {
                        ordered.pop_front();
                    }
                }
            }
        }
        // Cap is the most recent N *raw complete lines*, not unique keys.
        // Repeated duplicate appends can fill the window and drop older
        // distinct keys on reload.
        let keys = ordered.into_iter().collect();
        Ok(Self { path, keys })
    }

    #[cfg(test)]
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Memory-only check-and-insert. Critical section must not fsync.
    /// Returns `true` when this call reserved the key.
    pub fn reserve(&mut self, key: &str) -> bool {
        self.keys.insert(key.to_string())
    }

    pub fn unreserve(&mut self, key: &str) {
        self.keys.remove(key);
    }

    fn append_key(path: &Path, key: &str) -> std::io::Result<()> {
        // Serialize the full record even if write_all needs more than one write.
        // This lock is only held by blocking threads, never the intake loop.
        static APPEND_LOCK: Mutex<()> = Mutex::new(());
        let _append = APPEND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(format!("{key}\n").as_bytes())?;
        file.sync_all()?;
        Ok(())
    }
}

/// Append+fsync on a blocking thread. Caller must not hold the store mutex.
pub async fn persist_dedupe_key(path: PathBuf, key: String) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || DedupeStore::append_key(&path, &key))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e.to_string())))
}

/// Persist occupancy after the event was already handled. On I/O failure the
/// in-memory reservation stays — rollback is only valid for `InfraFailure`.
/// A crash between queue and persist is an accepted one-event window: restart
/// loses the HashSet, so a retry may enqueue again.
/// Sustained I/O failure opens a multi-event window: a `duplicate` ack from
/// memory alone lets the gateway advance while disk never recorded the keys,
/// and relay redelivery after restart would reprocess them. The duplicate
/// path therefore retries persist once before acking.
/// Residual shutdown window only: a duplicate-path persist retry can land on
/// disk while another connection's enqueue is still in flight; if that
/// enqueue then returns `InfraFailure` and unreserves, disk has the key and
/// memory does not. Reload treats the on-disk key as already processed.
/// `InfraFailure` fires only when the main loop is already gone. Not closed
/// in code (an in-flight marker would skip the duplicate retry).
pub async fn persist_occupancy(
    store: &Arc<Mutex<DedupeStore>>,
    key: String,
) -> std::io::Result<()> {
    let path = {
        let guard = store.lock().unwrap_or_else(|e| e.into_inner());
        guard.path().to_path_buf()
    };
    match persist_dedupe_key(path, key.clone()).await {
        Ok(()) => Ok(()),
        Err(e) => {
            tracing::warn!(key = %key, error = %e, "forward dedupe persist failed");
            Err(e)
        }
    }
}

#[derive(Clone)]
pub struct ForwardPolicy {
    pub peer_uid: u32,
    pub subscribed: Arc<Mutex<HashSet<Uuid>>>,
    pub store: Arc<Mutex<DedupeStore>>,
}

fn peer_uid(stream: &UnixStream) -> Result<u32> {
    #[cfg(target_os = "linux")]
    {
        let cred = nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
            .context("SO_PEERCRED")?;
        Ok(cred.uid())
    }
    #[cfg(target_os = "macos")]
    {
        let cred = nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::LocalPeerCred)
            .context("LOCAL_PEERCRED")?;
        Ok(cred.uid())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = stream;
        anyhow::bail!("peer credentials unsupported on this OS")
    }
}

#[cfg(test)]
fn current_uid() -> u32 {
    nix::unistd::Uid::current().as_raw()
}

fn log_status(event_id: &str, channel_id: &str, status: &str, reason: Option<&str>) {
    let ev8: String = event_id.chars().take(8).collect();
    let ch8: String = channel_id.chars().take(8).collect();
    match reason {
        Some(r) => {
            tracing::info!(event_id = %ev8, channel_id = %ch8, status, reason = r, "forward")
        }
        None => tracing::info!(event_id = %ev8, channel_id = %ch8, status, "forward"),
    }
}

fn encode_ack(event_id: &str, status: &str, reason: Option<&str>) -> Vec<u8> {
    let body = json!({
        "ack": event_id,
        "status": status,
        "reason": reason,
    });
    let blob = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = Vec::with_capacity(4 + blob.len());
    out.extend_from_slice(&(blob.len() as u32).to_be_bytes());
    out.extend_from_slice(&blob);
    out
}

async fn write_ack(stream: &mut UnixStream, event_id: &str, status: &str, reason: Option<&str>) {
    let bytes = encode_ack(event_id, status, reason);
    if let Err(e) = stream.write_all(&bytes).await {
        tracing::debug!(error = %e, "forward ack write failed (already accepted)");
    }
}

fn event_has_h_tag(event: &Event, channel_id: Uuid) -> bool {
    let expected = channel_id.to_string();
    event.tags.iter().any(|tag| {
        let v = tag.as_slice();
        v.len() >= 2 && v[0] == "h" && v[1] == expected
    })
}

enum ReadFrameError {
    Timeout,
    TooLarge,
    Bad,
}

async fn read_frame(stream: &mut UnixStream) -> std::result::Result<ForwardFrame, ReadFrameError> {
    let mut header = [0u8; 4];
    match tokio::time::timeout(ACK_TIMEOUT, stream.read_exact(&mut header)).await {
        Err(_) => return Err(ReadFrameError::Timeout),
        Ok(Err(_)) => return Err(ReadFrameError::Bad),
        Ok(Ok(_)) => {}
    }
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 {
        return Err(ReadFrameError::Bad);
    }
    if size > FORWARD_FRAME_MAX {
        return Err(ReadFrameError::TooLarge);
    }
    let mut payload = vec![0u8; size];
    match tokio::time::timeout(ACK_TIMEOUT, stream.read_exact(&mut payload)).await {
        Err(_) => return Err(ReadFrameError::Timeout),
        Ok(Err(_)) => return Err(ReadFrameError::Bad),
        Ok(Ok(_)) => {}
    }
    serde_json::from_slice(&payload).map_err(|_| ReadFrameError::Bad)
}

pub async fn serve_connection<F, Fut>(mut stream: UnixStream, policy: &ForwardPolicy, enqueue: F)
where
    F: FnOnce(BuzzEvent) -> Fut,
    Fut: Future<Output = EnqueueResult>,
{
    let uid = match peer_uid(&stream) {
        Ok(uid) => uid,
        Err(_) => {
            log_status("unknown", "unknown", "rejected", Some("peer_uid"));
            write_ack(&mut stream, "unknown", "rejected", Some("peer_uid")).await;
            return;
        }
    };
    if uid != policy.peer_uid {
        log_status("unknown", "unknown", "rejected", Some("peer_uid"));
        write_ack(&mut stream, "unknown", "rejected", Some("peer_uid")).await;
        return;
    }

    let frame = match read_frame(&mut stream).await {
        Ok(frame) => frame,
        Err(ReadFrameError::Timeout) => return,
        Err(ReadFrameError::TooLarge) => {
            log_status("unknown", "unknown", "rejected", Some("frame_too_large"));
            write_ack(&mut stream, "unknown", "rejected", Some("frame_too_large")).await;
            return;
        }
        Err(ReadFrameError::Bad) => {
            log_status("unknown", "unknown", "rejected", Some("bad_frame"));
            write_ack(&mut stream, "unknown", "rejected", Some("bad_frame")).await;
            return;
        }
    };

    let event_id_hint = frame
        .event
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let channel_hint = frame.channel_id.clone();

    if frame.direction != "inbound" {
        log_status(&event_id_hint, &channel_hint, "rejected", Some("bad_frame"));
        write_ack(&mut stream, &event_id_hint, "rejected", Some("bad_frame")).await;
        return;
    }
    let channel_id = match Uuid::parse_str(&frame.channel_id) {
        Ok(id) => id,
        Err(_) => {
            log_status(&event_id_hint, &channel_hint, "rejected", Some("bad_frame"));
            write_ack(&mut stream, &event_id_hint, "rejected", Some("bad_frame")).await;
            return;
        }
    };
    if frame.channel_id != channel_id.to_string() {
        log_status(&event_id_hint, &channel_hint, "rejected", Some("bad_frame"));
        write_ack(&mut stream, &event_id_hint, "rejected", Some("bad_frame")).await;
        return;
    }
    let event: Event = match serde_json::from_value(frame.event.clone()) {
        Ok(event) => event,
        Err(_) => {
            log_status(&event_id_hint, &channel_hint, "rejected", Some("bad_frame"));
            write_ack(&mut stream, &event_id_hint, "rejected", Some("bad_frame")).await;
            return;
        }
    };
    if event.verify().is_err() {
        log_status(
            &event_id_hint,
            &channel_hint,
            "rejected",
            Some("bad_signature"),
        );
        write_ack(
            &mut stream,
            &event_id_hint,
            "rejected",
            Some("bad_signature"),
        )
        .await;
        return;
    }
    let event_id = event.id.to_hex();
    if !event_has_h_tag(&event, channel_id) {
        log_status(
            &event_id,
            &frame.channel_id,
            "rejected",
            Some("channel_mismatch"),
        );
        write_ack(&mut stream, &event_id, "rejected", Some("channel_mismatch")).await;
        return;
    }
    let subscribed = policy
        .subscribed
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(&channel_id);
    if !subscribed {
        log_status(
            &event_id,
            &frame.channel_id,
            "rejected",
            Some("channel_not_subscribed"),
        );
        write_ack(
            &mut stream,
            &event_id,
            "rejected",
            Some("channel_not_subscribed"),
        )
        .await;
        return;
    }
    let kind = event.kind.as_u16() as u32;
    if matches!(
        kind,
        KIND_MEMBER_ADDED_NOTIFICATION
            | KIND_MEMBER_REMOVED_NOTIFICATION
            | KIND_AGENT_OBSERVER_FRAME
    ) {
        log_status(
            &event_id,
            &frame.channel_id,
            "rejected",
            Some("wake_denied"),
        );
        write_ack(&mut stream, &event_id, "rejected", Some("wake_denied")).await;
        return;
    }
    let channel_s = channel_id.to_string();
    let expected = compute_dedupe_key(&event_id, "inbound", &channel_s);
    if expected != frame.dedupe_key {
        log_status(
            &event_id,
            &frame.channel_id,
            "rejected",
            Some("dedupe_key_mismatch"),
        );
        write_ack(
            &mut stream,
            &event_id,
            "rejected",
            Some("dedupe_key_mismatch"),
        )
        .await;
        return;
    }

    let reserved = {
        let mut store = policy.store.lock().unwrap_or_else(|e| e.into_inner());
        store.reserve(&expected)
    };
    if !reserved {
        // Memory-only occupancy: retry persist once so a prior disk failure
        // does not ack duplicate while the key is still missing on disk.
        if persist_occupancy(&policy.store, expected.clone())
            .await
            .is_err()
        {
            return;
        }
        log_status(&event_id, &frame.channel_id, "duplicate", None);
        write_ack(&mut stream, &event_id, "duplicate", None).await;
        return;
    }

    // Forward frames have no relay connection generation. The shared author
    // gate refreshes relay identity for this sentinel before attribution.
    let buzz_event = BuzzEvent {
        connection_generation: u64::MAX,
        channel_id,
        event,
    };
    let outcome = enqueue(buzz_event).await;
    if outcome == EnqueueResult::InfraFailure {
        {
            let mut store = policy.store.lock().unwrap_or_else(|e| e.into_inner());
            store.unreserve(&expected);
        }
        return;
    }

    // Queued/Drop/Ignored already happened. Persist failure must not unreserve:
    // a gateway retry then hits the in-memory key and acks duplicate.
    if persist_occupancy(&policy.store, expected.clone())
        .await
        .is_err()
    {
        return;
    }

    match outcome {
        EnqueueResult::Queued | EnqueueResult::Ignored => {
            log_status(&event_id, &frame.channel_id, "accepted", None);
            write_ack(&mut stream, &event_id, "accepted", None).await;
        }
        EnqueueResult::Drop => {
            log_status(
                &event_id,
                &frame.channel_id,
                "duplicate",
                Some("queue_drop"),
            );
            write_ack(&mut stream, &event_id, "duplicate", Some("queue_drop")).await;
        }
        EnqueueResult::InfraFailure => {}
    }
}

pub fn bind_listener(path: &Path) -> Result<UnixListener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        anyhow::ensure!(meta.file_type().is_socket(), "forward path is not a socket");
        anyhow::ensure!(
            std::os::unix::net::UnixStream::connect(path).is_err(),
            "forward socket is already listening"
        );
        std::fs::remove_file(path)
            .with_context(|| format!("remove stale socket {}", path.display()))?;
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    // Never remove a pre-existing temporary path: it may be unrelated data.
    // bind fails closed on any collision, including a dangling symlink.
    let listener = UnixListener::bind(&tmp).with_context(|| format!("bind {}", tmp.display()))?;
    if let Err(e) = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o660)) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("chmod {}", tmp.display()));
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("rename {} -> {}", tmp.display(), path.display()));
    }
    Ok(listener)
}

pub async fn accept_loop(
    listener: UnixListener,
    policy: ForwardPolicy,
    work_tx: mpsc::Sender<ForwardWork>,
) {
    let slots = Arc::new(tokio::sync::Semaphore::new(32));
    loop {
        let Ok(slot) = slots.clone().acquire_owned().await else {
            return;
        };
        let (stream, _) = match listener.accept().await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(error = %e, "forward accept failed");
                continue;
            }
        };
        let policy = policy.clone();
        let work_tx = work_tx.clone();
        tokio::spawn(async move {
            let _slot = slot;
            serve_connection(stream, &policy, |event| {
                let work_tx = work_tx.clone();
                async move {
                    let (reply, rx) = oneshot::channel();
                    if work_tx.send(ForwardWork { event, reply }).await.is_err() {
                        return EnqueueResult::InfraFailure;
                    }
                    rx.await.unwrap_or(EnqueueResult::InfraFailure)
                }
            })
            .await;
        });
    }
}

#[cfg(test)]
pub fn relay_event_is_duplicate(store: &DedupeStore, event: &Event, channel_id: Uuid) -> bool {
    store.contains(&relay_dedupe_key(event, channel_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::kind::KIND_STREAM_MESSAGE;
    use nostr::{EventBuilder, Keys, Kind, Tag};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::AsyncWriteExt;

    static SOCK_SEQ: AtomicU64 = AtomicU64::new(1);

    fn sock_path() -> PathBuf {
        PathBuf::from(format!(
            "/tmp/bz-fwd-{}-{}.sock",
            std::process::id(),
            SOCK_SEQ.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn signed_event(keys: &Keys, kind: u32, content: &str, channel: Uuid) -> Event {
        let h = Tag::parse(["h", &channel.to_string()]).expect("h tag");
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags([h])
            .sign_with_keys(keys)
            .expect("sign")
    }

    fn frame_for(event: &Event, channel: Uuid, direction: &str) -> Value {
        let event_id = event.id.to_hex();
        let channel_s = channel.to_string();
        json!({
            "event": serde_json::to_value(event).unwrap(),
            "direction": direction,
            "chat_type": "group",
            "channel_id": channel_s,
            "dedupe_key": compute_dedupe_key(&event_id, direction, &channel_s),
        })
    }

    fn encode_frame(value: &Value) -> Vec<u8> {
        let blob = serde_json::to_vec(value).unwrap();
        let mut out = Vec::with_capacity(4 + blob.len());
        out.extend_from_slice(&(blob.len() as u32).to_be_bytes());
        out.extend_from_slice(&blob);
        out
    }

    async fn read_ack(stream: &mut UnixStream) -> Value {
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await.expect("ack header");
        let size = u32::from_be_bytes(header) as usize;
        let mut payload = vec![0u8; size];
        stream.read_exact(&mut payload).await.expect("ack body");
        serde_json::from_slice(&payload).expect("ack json")
    }

    struct Harness {
        path: PathBuf,
        store: Arc<Mutex<DedupeStore>>,
        queue: Arc<Mutex<Vec<BuzzEvent>>>,
        _listener: UnixListener,
        enqueue: EnqueueResult,
        policy: ForwardPolicy,
    }

    impl Harness {
        fn new(
            peer_uid: u32,
            subscribed: HashSet<Uuid>,
            _self_keys: &Keys,
            enqueue: EnqueueResult,
        ) -> Self {
            let path = sock_path();
            let state = path.with_extension("jsonl");
            let store = Arc::new(Mutex::new(DedupeStore::load(&state).unwrap()));
            let listener = bind_listener(&path).unwrap();
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o660);
            let policy = ForwardPolicy {
                peer_uid,
                subscribed: Arc::new(Mutex::new(subscribed)),
                store: store.clone(),
            };
            Self {
                path,
                store,
                queue: Arc::new(Mutex::new(Vec::new())),
                _listener: listener,
                enqueue,
                policy,
            }
        }

        fn block_persist(&self) {
            let state = self.store.lock().unwrap().path().to_path_buf();
            let _ = std::fs::remove_file(&state);
            std::fs::create_dir(&state).expect("block persist path");
        }

        fn unblock_persist(&self) {
            let state = self.store.lock().unwrap().path().to_path_buf();
            let _ = std::fs::remove_dir(&state);
        }

        async fn accept_one(&self) {
            let (stream, _) = self._listener.accept().await.expect("accept");
            let queue = self.queue.clone();
            let enqueue = self.enqueue;
            serve_connection(stream, &self.policy, move |event| {
                let queue = queue.clone();
                async move {
                    if enqueue == EnqueueResult::Queued {
                        queue.lock().unwrap().push(event);
                    }
                    enqueue
                }
            })
            .await;
        }

        async fn exchange(&self, frame: Value) -> Value {
            let mut client = UnixStream::connect(&self.path).await.expect("connect");
            let accept = self.accept_one();
            let send = async {
                client.write_all(&encode_frame(&frame)).await.unwrap();
                read_ack(&mut client).await
            };
            let (_, ack) = tokio::join!(accept, send);
            ack
        }
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let state = self.path.with_extension("jsonl");
            let _ = std::fs::remove_file(&state);
            let _ = std::fs::remove_dir(&state);
        }
    }

    #[tokio::test]
    async fn accepted_frame_queues_and_persists() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["status"], "accepted");
        assert_eq!(ack["ack"], event.id.to_hex());
        assert_eq!(h.queue.lock().unwrap().len(), 1);
        assert_eq!(h.store.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn second_identical_frame_is_duplicate() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let frame = frame_for(&event, channel, "inbound");
        let first = h.exchange(frame.clone()).await;
        let second = h.exchange(frame).await;
        assert_eq!(first["status"], "accepted");
        assert_eq!(second["status"], "duplicate");
        assert_eq!(h.queue.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn concurrent_same_key_one_accepted() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let frame = frame_for(&event, channel, "inbound");
        let mut c1 = UnixStream::connect(&h.path).await.unwrap();
        let mut c2 = UnixStream::connect(&h.path).await.unwrap();
        let accept = async {
            h.accept_one().await;
            h.accept_one().await;
        };
        let send = async {
            let bytes = encode_frame(&frame);
            c1.write_all(&bytes).await.unwrap();
            c2.write_all(&bytes).await.unwrap();
            let a = read_ack(&mut c1).await;
            let b = read_ack(&mut c2).await;
            (a, b)
        };
        let (_, (a, b)) = tokio::join!(accept, send);
        let statuses = [a["status"].as_str().unwrap(), b["status"].as_str().unwrap()];
        assert!(statuses.contains(&"accepted"));
        assert!(statuses.contains(&"duplicate"));
        assert_eq!(h.queue.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn restart_reloads_store() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let frame = frame_for(&event, channel, "inbound");
        assert_eq!(h.exchange(frame.clone()).await["status"], "accepted");
        let reloaded = DedupeStore::load(h.path.with_extension("jsonl")).unwrap();
        assert!(relay_event_is_duplicate(&reloaded, &event, channel));
        *h.store.lock().unwrap() = reloaded;
        assert_eq!(h.exchange(frame).await["status"], "duplicate");
    }

    #[tokio::test]
    async fn bad_signature_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let mut frame = frame_for(&event, channel, "inbound");
        frame["event"]["sig"] = json!("0".repeat(128));
        let ack = h.exchange(frame).await;
        assert_eq!(ack["status"], "rejected");
        assert_eq!(ack["reason"], "bad_signature");
        assert!(h.queue.lock().unwrap().is_empty());
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn membership_set_change_is_visible_to_forward_gate() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(current_uid(), HashSet::new(), &seat, EnqueueResult::Queued);
        let first = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let ack = h.exchange(frame_for(&first, channel, "inbound")).await;
        assert_eq!(ack["reason"], "channel_not_subscribed");

        h.policy.subscribed.lock().unwrap().insert(channel);
        let opened = signed_event(&keys, KIND_STREAM_MESSAGE, "opened", channel);
        let ack = h.exchange(frame_for(&opened, channel, "inbound")).await;
        assert_eq!(ack["status"], "accepted");

        h.policy.subscribed.lock().unwrap().remove(&channel);
        let closed = signed_event(&keys, KIND_STREAM_MESSAGE, "closed", channel);
        let ack = h.exchange(frame_for(&closed, channel, "inbound")).await;
        assert_eq!(ack["reason"], "channel_not_subscribed");
    }

    #[tokio::test]
    async fn unsubscribed_channel_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(current_uid(), HashSet::new(), &seat, EnqueueResult::Queued);
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["reason"], "channel_not_subscribed");
        assert!(h.queue.lock().unwrap().is_empty());
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn membership_kind_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, 44100, "membership", channel);
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["reason"], "wake_denied");
        assert!(h.queue.lock().unwrap().is_empty());
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn dedupe_key_mismatch_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let mut frame = frame_for(&event, channel, "inbound");
        frame["dedupe_key"] = json!("ab".repeat(32));
        let ack = h.exchange(frame).await;
        assert_eq!(ack["reason"], "dedupe_key_mismatch");
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn self_echo_reaches_common_ignore_self_policy() {
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&seat, KIND_STREAM_MESSAGE, "loop", channel);
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["status"], "accepted");
        assert_eq!(h.queue.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn peer_uid_mismatch_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid().wrapping_add(1),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["reason"], "peer_uid");
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn queue_drop_reports_duplicate() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Drop,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["status"], "duplicate");
        assert_eq!(ack["reason"], "queue_drop");
        assert_eq!(h.store.lock().unwrap().len(), 1);
        assert!(h.queue.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ack_write_failure_keeps_occupancy() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let frame = frame_for(&event, channel, "inbound");
        let mut client = UnixStream::connect(&h.path).await.unwrap();
        let accept = h.accept_one();
        let send = async {
            client.write_all(&encode_frame(&frame)).await.unwrap();
            drop(client);
        };
        tokio::join!(accept, send);
        assert_eq!(h.store.lock().unwrap().len(), 1);
        assert_eq!(h.queue.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn frame_too_large_rejected() {
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let mut client = UnixStream::connect(&h.path).await.unwrap();
        let accept = h.accept_one();
        let send = async {
            let size = (FORWARD_FRAME_MAX as u32) + 1;
            client.write_all(&size.to_be_bytes()).await.unwrap();
            client.write_all(&[0u8; 8]).await.ok();
            read_ack(&mut client).await
        };
        let (_, ack) = tokio::join!(accept, send);
        assert_eq!(ack["reason"], "frame_too_large");
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn h_tag_from_another_channel_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let subscribed = Uuid::new_v4();
        let other = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([subscribed]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", other);
        let ack = h.exchange(frame_for(&event, subscribed, "inbound")).await;
        assert_eq!(ack["status"], "rejected");
        assert_eq!(ack["reason"], "channel_mismatch");
        assert!(h.queue.lock().unwrap().is_empty());
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn relay_first_then_forward_is_duplicate() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let key = relay_dedupe_key(&event, channel);
        {
            let mut store = h.store.lock().unwrap();
            assert!(store.reserve(&key));
        }
        let persist_path = h.store.lock().unwrap().path().to_path_buf();
        persist_dedupe_key(persist_path, key).await.unwrap();
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["status"], "duplicate");
        assert!(ack["reason"].is_null());
        assert!(h.queue.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn forward_first_then_relay_skips() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["status"], "accepted");
        let store = h.store.lock().unwrap();
        assert!(relay_event_is_duplicate(&store, &event, channel));
        assert!(!{
            let mut clone = store.clone();
            clone.reserve(&relay_dedupe_key(&event, channel))
        });
    }

    #[tokio::test]
    async fn noncanonical_channel_id_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let variants = [
            channel.to_string().to_uppercase(),
            format!("{{{channel}}}"),
            format!("urn:uuid:{channel}"),
        ];
        for raw in variants {
            let mut frame = frame_for(&event, channel, "inbound");
            frame["channel_id"] = json!(raw);
            let ack = h.exchange(frame).await;
            assert_eq!(ack["reason"], "bad_frame", "raw channel_id should reject");
        }
        assert!(h.queue.lock().unwrap().is_empty());
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn outbound_direction_rejected() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let ack = h.exchange(frame_for(&event, channel, "outbound")).await;
        assert_eq!(ack["status"], "rejected");
        assert_eq!(ack["reason"], "bad_frame");
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn non_json_payload_is_bad_frame() {
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let mut client = UnixStream::connect(&h.path).await.unwrap();
        let accept = h.accept_one();
        let send = async {
            let blob = b"not-json";
            let mut out = Vec::new();
            out.extend_from_slice(&(blob.len() as u32).to_be_bytes());
            out.extend_from_slice(blob);
            client.write_all(&out).await.unwrap();
            read_ack(&mut client).await
        };
        let (_, ack) = tokio::join!(accept, send);
        assert_eq!(ack["reason"], "bad_frame");
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn connect_and_send_nothing_times_out_without_ack() {
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let mut client = UnixStream::connect(&h.path).await.unwrap();
        let accept = h.accept_one();
        let send = async {
            let mut header = [0u8; 4];
            let res = tokio::time::timeout(
                ACK_TIMEOUT + Duration::from_secs(2),
                client.read_exact(&mut header),
            )
            .await;
            if let Ok(Ok(_)) = res {
                panic!("timeout path must not send an ack");
            }
        };
        tokio::join!(accept, send);
        assert_eq!(h.store.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn infra_failure_closes_without_ack() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::InfraFailure,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let frame = frame_for(&event, channel, "inbound");
        let mut client = UnixStream::connect(&h.path).await.unwrap();
        let accept = h.accept_one();
        let send = async {
            client.write_all(&encode_frame(&frame)).await.unwrap();
            let mut header = [0u8; 4];
            let res =
                tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut header)).await;
            if let Ok(Ok(_)) = res {
                panic!("infra failure must not ack");
            }
        };
        tokio::join!(accept, send);
        assert_eq!(h.store.lock().unwrap().len(), 0);
        assert!(h.queue.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn persist_fail_after_queued_keeps_reservation() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        h.block_persist();
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let frame = frame_for(&event, channel, "inbound");
        let mut client = UnixStream::connect(&h.path).await.unwrap();
        let accept = h.accept_one();
        let send = async {
            client.write_all(&encode_frame(&frame)).await.unwrap();
            let mut header = [0u8; 4];
            let res =
                tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut header)).await;
            if let Ok(Ok(_)) = res {
                panic!("persist failure after queued must not ack");
            }
        };
        tokio::join!(accept, send);
        assert_eq!(h.store.lock().unwrap().len(), 1);
        assert_eq!(h.queue.lock().unwrap().len(), 1);
        let mut retry = UnixStream::connect(&h.path).await.unwrap();
        let accept_retry = h.accept_one();
        let send_retry = async {
            retry.write_all(&encode_frame(&frame)).await.unwrap();
            let mut header = [0u8; 4];
            let res =
                tokio::time::timeout(Duration::from_secs(2), retry.read_exact(&mut header)).await;
            if let Ok(Ok(_)) = res {
                panic!("duplicate persist retry still failing must not ack");
            }
        };
        tokio::join!(accept_retry, send_retry);
        h.unblock_persist();
        let ack = h.exchange(frame).await;
        assert_eq!(ack["status"], "duplicate");
        assert_eq!(h.queue.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn persist_fail_after_relay_reserve_forward_is_duplicate() {
        let keys = Keys::generate();
        let seat = Keys::generate();
        let channel = Uuid::new_v4();
        let h = Harness::new(
            current_uid(),
            HashSet::from([channel]),
            &seat,
            EnqueueResult::Queued,
        );
        let event = signed_event(&keys, KIND_STREAM_MESSAGE, "hello", channel);
        let key = relay_dedupe_key(&event, channel);
        {
            let mut store = h.store.lock().unwrap();
            assert!(store.reserve(&key));
        }
        h.block_persist();
        let err = persist_occupancy(&h.store, key.clone()).await;
        assert!(err.is_err());
        assert_eq!(h.store.lock().unwrap().len(), 1);
        assert!(relay_event_is_duplicate(
            &h.store.lock().unwrap(),
            &event,
            channel
        ));
        h.unblock_persist();
        let ack = h.exchange(frame_for(&event, channel, "inbound")).await;
        assert_eq!(ack["status"], "duplicate");
        assert!(h.queue.lock().unwrap().is_empty());
    }

    #[test]
    fn load_survives_truncated_last_line() {
        let path = PathBuf::from(format!(
            "/tmp/bz-dedupe-trunc-{}-{}.jsonl",
            std::process::id(),
            SOCK_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let full = "ab".repeat(32);
        std::fs::write(&path, format!("{full}\ntruncated")).unwrap();
        let store = DedupeStore::load(&path).unwrap();
        assert!(store.contains(&full));
        assert!(!store.contains("truncated"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_caps_memory_to_last_n_keys() {
        let path = PathBuf::from(format!(
            "/tmp/bz-dedupe-cap-{}-{}.jsonl",
            std::process::id(),
            SOCK_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let k1 = "aa".repeat(32);
        let k2 = "bb".repeat(32);
        let k3 = "cc".repeat(32);
        std::fs::write(&path, format!("{k1}\n{k2}\n{k3}\n")).unwrap();
        let store = DedupeStore::load_with_cap(&path, 2).unwrap();
        assert!(!store.contains(&k1));
        assert!(store.contains(&k2));
        assert!(store.contains(&k3));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn bind_preserves_regular_files_and_temporary_collisions() {
        let path = PathBuf::from(format!("/tmp/bz-bind-{}.sock", Uuid::new_v4()));
        std::fs::write(&path, b"keep").unwrap();
        assert!(bind_listener(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
        std::fs::remove_file(&path).unwrap();
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&tmp, b"also keep").unwrap();
        assert!(bind_listener(&path).is_err());
        assert_eq!(std::fs::read(&tmp).unwrap(), b"also keep");
        std::fs::remove_file(&tmp).unwrap();
    }

    #[tokio::test]
    async fn bind_refuses_to_replace_live_listener() {
        let path = PathBuf::from(format!("/tmp/bz-live-{}.sock", Uuid::new_v4()));
        let listener = bind_listener(&path).unwrap();
        assert!(bind_listener(&path).is_err());
        assert!(UnixStream::connect(&path).await.is_ok());
        drop(listener);
        std::fs::remove_file(&path).unwrap();
    }

    #[tokio::test]
    async fn concurrent_appends_reload_every_complete_key() {
        let path = PathBuf::from(format!("/tmp/bz-append-{}.jsonl", Uuid::new_v4()));
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..128 {
            tasks.spawn(persist_dedupe_key(path.clone(), format!("{i:064x}")));
        }
        while let Some(result) = tasks.join_next().await {
            result.unwrap().unwrap();
        }
        let store = DedupeStore::load(&path).unwrap();
        assert_eq!(store.len(), 128);
        for i in 0..128 {
            assert!(store.contains(&format!("{i:064x}")));
        }
        std::fs::remove_file(path).unwrap();
    }
}
