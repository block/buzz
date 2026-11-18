use std::time::Duration;

use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::time::Instant;

/// Inbound text frames are coalesced into one `Channel::send` for this long
/// before delivery. Collapses N main-run-loop wakeups into one under a
/// catch-up storm without adding latency the relay protocol can observe.
pub(crate) const BATCH_WINDOW: Duration = Duration::from_millis(8);
/// Byte ceiling for a coalesced batch, measured on the *serialized* payload.
///
/// `tauri::ipc::Channel::send` forks on payload size: below
/// `MAX_JSON_DIRECT_EXECUTE_THRESHOLD` (8192) it goes straight to
/// `webview.eval`; at or above it the body is parked in a `ChannelDataIpcQueue`
/// and the webview is made to call *back* into Rust over the IPC to fetch it
/// (tauri-2.11.5 `src/ipc/channel.rs:37,154-181,319-331`). That round-trip is
/// what batching is supposed to remove, so a batch must never cross the line —
/// bounding by frame count instead would put every batch on the slow path.
/// The margin absorbs the envelope; the check itself uses real serialized
/// length, because JSON escaping inflates payloads by an amount no fixed
/// per-frame estimate can bound.
pub(crate) const BATCH_MAX_SERIALIZED_BYTES: usize = 7680;

/// Coalesces inbound frames into a single IPC delivery.
///
/// Frames are serialized once on arrival so the batch can be bounded by its
/// true serialized length, and are concatenated into a JSON array at flush —
/// no value is serialized twice. Every delivery is an array, including the
/// single-frame case; the renderer accepts both shapes.
#[derive(Default)]
pub(crate) struct FrameBatch {
    frames: Vec<String>,
    /// Serialized length of the delivered array, kept in sync with `frames`:
    /// the enclosing brackets plus each frame and its separating comma.
    serialized_len: usize,
    deadline: Option<Instant>,
}

impl FrameBatch {
    /// Serialized length of the array if `frame` were appended.
    pub(crate) fn projected_len(&self, frame: &str) -> usize {
        let separator = usize::from(!self.frames.is_empty());
        self.serialized_len.max(2) + separator + frame.len()
    }

    pub(crate) fn push(&mut self, frame: String) {
        self.serialized_len = self.projected_len(&frame);
        self.frames.push(frame);
        self.deadline
            .get_or_insert_with(|| Instant::now() + BATCH_WINDOW);
    }

    /// Resolves when the open batch is due, or never while there is none.
    pub(crate) async fn due(&self) {
        match self.deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    }

    pub(crate) fn flush(&mut self, on_message: &Channel<InvokeResponseBody>) {
        if self.frames.is_empty() {
            return;
        }
        let payload = format!("[{}]", self.frames.join(","));
        self.frames.clear();
        self.serialized_len = 0;
        self.deadline = None;
        let _ = on_message.send(InvokeResponseBody::Json(payload));
    }
}

/// Whether a relay frame must reach the renderer without waiting out the batch
/// window. Only the NIP-42 challenge qualifies: it gates a round trip the
/// relay is waiting on, whereas `OK`/`EOSE` ride the window so catch-up
/// batching survives.
///
/// Takes the relay payload, not the serialized envelope — inside the envelope
/// the payload's quotes are escaped and no plain `"AUTH"` prefix exists.
///
/// Conservative by construction — a missed match costs at most one batch
/// window of latency against a 25s auth timeout, never correctness.
pub(crate) fn is_auth_challenge(payload: &str) -> bool {
    payload
        .trim_start()
        .strip_prefix('[')
        .unwrap_or_default()
        .trim_start()
        .starts_with("\"AUTH\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Records each delivery as its raw JSON payload, so tests assert on what
    /// the renderer actually receives rather than on internal batch state.
    fn recording_channel() -> (
        Channel<InvokeResponseBody>,
        Arc<std::sync::Mutex<Vec<String>>>,
    ) {
        // A std mutex: `Channel::send` is synchronous and runs on whatever
        // thread flushed, including inside the async runtime.
        let deliveries = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = deliveries.clone();
        let channel = Channel::new(move |body: InvokeResponseBody| {
            let payload = match body {
                InvokeResponseBody::Json(json) => json,
                InvokeResponseBody::Raw(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            };
            sink.lock().unwrap().push(payload);
            Ok(())
        });
        (channel, deliveries)
    }

    /// Mirrors the envelope `native_websocket` serializes, so these tests bind
    /// to the real wire shape rather than a convenient stand-in.
    fn text_frame(payload: &str) -> String {
        serde_json::json!({ "type": "Text", "data": payload }).to_string()
    }

    #[test]
    fn batch_bound_tracks_real_serialized_length() {
        let mut batch = FrameBatch::default();
        let first = text_frame("one");
        let second = text_frame("two");
        batch.push(first.clone());
        batch.push(second.clone());

        // The tracked length must equal the payload actually built at flush;
        // an estimate that drifts from it would silently cross the 8192 fork.
        let expected = format!("[{first},{second}]");
        assert_eq!(batch.serialized_len, expected.len());
    }

    #[test]
    fn escape_heavy_frames_stay_under_the_direct_eval_threshold() {
        // Quotes double under JSON escaping, so a bound applied to raw relay
        // bytes would pass here while the serialized body crosses 8192 and
        // silently moves every batch onto the fetch round-trip.
        let mut batch = FrameBatch::default();
        let mut pushed = 0;
        loop {
            let frame = text_frame(&"\"".repeat(512));
            if batch.projected_len(&frame) > BATCH_MAX_SERIALIZED_BYTES {
                break;
            }
            batch.push(frame);
            pushed += 1;
        }

        assert!(
            pushed > 0,
            "bound must admit at least one escape-heavy frame"
        );
        assert!(
            batch.serialized_len < 8192,
            "serialized batch {} must stay under the direct-eval threshold",
            batch.serialized_len
        );
    }

    #[tokio::test]
    async fn frames_within_the_window_arrive_as_one_delivery() {
        let (channel, deliveries) = recording_channel();
        let mut batch = FrameBatch::default();
        batch.push(text_frame("one"));
        batch.push(text_frame("two"));
        batch.push(text_frame("three"));
        batch.flush(&channel);

        let deliveries = deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1, "three frames must cost one IPC wakeup");
        // Asserted as the wire shape the renderer parses, not as a Rust type.
        let frames: Vec<serde_json::Value> = serde_json::from_str(&deliveries[0]).unwrap();
        let texts: Vec<&str> = frames
            .iter()
            .map(|frame| frame["data"].as_str().expect("text frame carries data"))
            .collect();
        assert_eq!(texts, ["one", "two", "three"], "FIFO order is preserved");
    }

    #[tokio::test]
    async fn oversize_frame_is_delivered_alone_without_stranding_predecessors() {
        let (channel, deliveries) = recording_channel();
        let mut batch = FrameBatch::default();
        batch.push(text_frame("small"));

        // A frame that cannot share a batch must flush what is buffered first,
        // then travel alone — the straddle case.
        let oversize = text_frame(&"x".repeat(BATCH_MAX_SERIALIZED_BYTES));
        assert!(batch.projected_len(&oversize) > BATCH_MAX_SERIALIZED_BYTES);
        batch.flush(&channel);
        batch.push(oversize);
        batch.flush(&channel);

        let deliveries = deliveries.lock().unwrap();
        assert_eq!(
            deliveries.len(),
            2,
            "predecessor must not ride the oversize batch"
        );
        let first: Vec<serde_json::Value> = serde_json::from_str(&deliveries[0]).unwrap();
        assert_eq!(first.len(), 1);
        let second: Vec<serde_json::Value> = serde_json::from_str(&deliveries[1]).unwrap();
        assert_eq!(second.len(), 1);
        assert!(deliveries[1].len() >= BATCH_MAX_SERIALIZED_BYTES);
    }

    #[tokio::test]
    async fn auth_challenge_flushes_immediately_and_keeps_earlier_frames_ahead_of_it() {
        let (channel, deliveries) = recording_channel();
        let auth_payload = serde_json::json!(["AUTH", "challenge"]).to_string();
        assert!(is_auth_challenge(&auth_payload));

        let mut batch = FrameBatch::default();
        batch.push(text_frame("earlier"));
        batch.push(text_frame(&auth_payload));
        batch.flush(&channel);

        let deliveries = deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        let frames: Vec<serde_json::Value> = serde_json::from_str(&deliveries[0]).unwrap();
        assert_eq!(
            frames.len(),
            2,
            "AUTH carries buffered frames with it, in order"
        );
        assert_eq!(
            frames[0]["data"], "earlier",
            "buffered frame stays ahead of AUTH"
        );
    }

    #[test]
    fn only_the_auth_challenge_bypasses_the_batch_window() {
        // OK and EOSE must ride the timer, or catch-up batching collapses back
        // to one delivery per frame.
        for payload in [
            serde_json::json!(["OK", "id", true, ""]).to_string(),
            serde_json::json!(["EOSE", "sub"]).to_string(),
            serde_json::json!(["EVENT", "sub", {"content": "AUTH"}]).to_string(),
            serde_json::json!(["NOTICE", "AUTH required"]).to_string(),
        ] {
            assert!(
                !is_auth_challenge(&payload),
                "{payload} must not force a flush"
            );
        }

        // The serialized envelope escapes the payload's quotes, so matching
        // against it would never fire — the bug this pair pins down.
        let envelope = text_frame(r#"["AUTH","c"]"#);
        assert!(!is_auth_challenge(&envelope));
    }

    #[tokio::test]
    async fn empty_batch_never_wakes_the_renderer() {
        let (channel, deliveries) = recording_channel();
        FrameBatch::default().flush(&channel);
        assert!(deliveries.lock().unwrap().is_empty());
    }
}
