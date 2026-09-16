//! One bounded ciphertext owner. No detached retries and no durable queue.
use super::*;
use std::collections::VecDeque;
use tokio::time::Instant;

// Includes raw serialization (tags, sig, ciphertext) AND matched scope bytes.
// Above the 87,472-byte NIP44 envelope, below arbitrary transport JSON limits.
// Oversized input is explicitly rejected at this native archive admission seam.
pub(super) const MAX_CANDIDATE_BYTES: usize = 128 * 1024;
const MAX_PARKED: usize = FLUSH_BATCH_SIZE + 256;
const MAX_PARKED_BYTES: usize = MAX_PARKED * MAX_CANDIDATE_BYTES;
const RETRY_INITIAL: Duration = Duration::from_secs(1);
const MAX_ATTEMPTS: u8 = 4; // initial attempt, then retries after 1, 2 and 4 seconds.

fn bytes(c: &ArchiveCandidate) -> usize {
    c.raw_event_json.len()
        + c.matched_scope.scope_value.len()
        + c.matched_scope.scope_type.as_str().len()
}
fn bounded(c: &ArchiveCandidate) -> bool {
    bytes(c) <= MAX_CANDIDATE_BYTES
}

// All retained states share one count/byte budget, including the in-flight
// originals. Exhaustion preserves ciphertext in memory until joined teardown;
// it never records the event as invalid or processed. Capacity rejection is
// explicit loss, unavoidable for ephemeral input during an indefinite outage.
struct RetryItem {
    candidate: ArchiveCandidate,
    attempts: u8,
    due: Instant,
}
#[derive(Default)]
struct Buffer {
    fresh: VecDeque<ArchiveCandidate>,
    retry: VecDeque<RetryItem>,
    exhausted: Vec<ArchiveCandidate>,
    count: usize,
    bytes: usize,
    fresh_deadline: Option<Instant>,
    reported_capacity: bool,
    prefer_fresh: bool,
}
impl Buffer {
    fn admit<I: ArchiveSyncIo + ?Sized>(&mut self, io: &I, c: ArchiveCandidate) {
        if !bounded(&c) || self.count >= MAX_PARKED || self.bytes + bytes(&c) > MAX_PARKED_BYTES {
            // Coalesce the capacity incident per run; do not flood IPC/logs.
            if !self.reported_capacity {
                io.report_degraded("capacity-exceeded-input-not-retained", 1);
                self.reported_capacity = true;
            }
            return;
        }
        self.count += 1;
        self.bytes += bytes(&c);
        if self.fresh.is_empty() {
            self.fresh_deadline = Some(Instant::now() + FLUSH_DEADLINE);
        }
        self.fresh.push_back(c);
    }
    fn next_deadline(&self) -> Option<Instant> {
        self.fresh_deadline
            .into_iter()
            .chain(self.retry.front().map(|r| r.due))
            .min()
    }
    fn take_job(&mut self) -> Option<(Vec<ArchiveCandidate>, u8)> {
        // Alternate a due retry with fresh work by rescheduling failed retries
        // strictly into the future. No new arrival can reset an old deadline.
        let fresh_ready = self.fresh.len() >= FLUSH_BATCH_SIZE
            || self.fresh_deadline.is_some_and(|d| d <= Instant::now());
        if !(self.prefer_fresh && fresh_ready)
            && self.retry.front().is_some_and(|r| r.due <= Instant::now())
        {
            self.prefer_fresh = true;
            let r = self.retry.pop_front()?;
            return Some((vec![r.candidate], r.attempts));
        }
        if fresh_ready {
            self.prefer_fresh = false;
            let n = self.fresh.len().min(FLUSH_BATCH_SIZE);
            let batch = self.fresh.drain(..n).collect();
            if self.fresh.is_empty() {
                self.fresh_deadline = None;
            }
            return Some((batch, 0));
        }
        None
    }
    fn complete<I: ArchiveSyncIo + ?Sized>(
        &mut self,
        io: &I,
        original_count: usize,
        original_bytes: usize,
        attempts: u8,
        outcome: SyncOutcome,
    ) {
        if outcome.committed.persisted_agent_metrics > 0 {
            io.notify_agent_metrics_changed();
        }
        self.count -= original_count;
        self.bytes -= original_bytes;
        let attempts = attempts + 1;
        let mut exhausted = 0;
        for candidate in outcome.retry {
            // Bucket expansion may return more candidates than were submitted;
            // the outcome boundary must enforce the budget too.
            if !bounded(&candidate)
                || self.count >= MAX_PARKED
                || self.bytes + bytes(&candidate) > MAX_PARKED_BYTES
            {
                if !self.reported_capacity {
                    io.report_degraded("capacity-exceeded-input-not-retained", 1);
                    self.reported_capacity = true;
                }
                continue;
            }
            self.count += 1;
            self.bytes += bytes(&candidate);
            if attempts >= MAX_ATTEMPTS {
                self.exhausted.push(candidate);
                exhausted += 1;
            } else {
                self.retry.push_back(RetryItem {
                    candidate,
                    attempts,
                    due: Instant::now() + RETRY_INITIAL * (1 << (attempts - 1)),
                });
            }
        }
        // Different attempt levels have different deadlines.
        self.retry.make_contiguous().sort_by_key(|r| r.due);
        if exhausted > 0 {
            io.report_degraded("retry-exhausted-ciphertext-retained", exhausted);
        }
    }
    fn finish(self) -> Vec<ArchiveCandidate> {
        self.exhausted
            .into_iter()
            .chain(self.retry.into_iter().map(|r| r.candidate))
            .chain(self.fresh)
            .collect()
    }
}

#[cfg(test)]
pub(super) async fn run_sync<I: ArchiveSyncIo + ?Sized>(
    io: &I,
    reload: Arc<Notify>,
    events: mpsc::Receiver<MatchedEvent>,
    cancel: CancellationToken,
) -> Vec<ArchiveCandidate> {
    run_sync_pending(io, reload, events, cancel, Vec::new()).await
}

pub(super) async fn run_sync_pending<I: ArchiveSyncIo + ?Sized>(
    io: &I,
    reload: Arc<Notify>,
    mut events: mpsc::Receiver<MatchedEvent>,
    cancel: CancellationToken,
    initial: Vec<ArchiveCandidate>,
) -> Vec<ArchiveCandidate> {
    let mut scopes = HashMap::new();
    let mut buffer = Buffer::default();
    for c in initial {
        buffer.admit(io, c);
    }
    tokio::select! { biased;
        _ = cancel.cancelled() => {},
        _ = io.invalidated() => { cancel.cancel(); },
        _ = reconcile(io, &mut scopes) => {},
    }
    loop {
        if cancel.is_cancelled() {
            break;
        }
        if let Some((batch, attempts)) = buffer.take_job() {
            let count = batch.len();
            let size = batch.iter().map(bytes).sum();
            // Single owned operation, never detached or dropped. Continue
            // draining intake while crypto/SQLite is pending, within the same
            // budget (in-flight originals remain charged until completion).
            let work = io.archive_retry(batch);
            tokio::pin!(work);
            let mut stopping = false;
            let outcome = loop {
                tokio::select! { biased;
                    result = &mut work => break result,
                    _ = cancel.cancelled(), if !stopping => { stopping = true; events.close(); },
                    _ = io.invalidated(), if !stopping => { stopping = true; cancel.cancel(); events.close(); },
                    received = events.recv(), if !stopping => match received {
                        Some(event) => if let Some(c) = candidate(event, &scopes) { buffer.admit(io, c); },
                        None => { stopping = true; },
                    },
                }
            };
            buffer.complete(io, count, size, attempts, outcome);
            if stopping {
                break;
            }
            continue;
        }
        let deadline = buffer
            .next_deadline()
            .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
        tokio::select! { biased;
            _ = cancel.cancelled() => break,
            _ = io.invalidated() => { cancel.cancel(); break; },
            _ = tokio::time::sleep_until(deadline), if buffer.next_deadline().is_some() => {},
            _ = reload.notified() => {
                tokio::select! { biased;
                    _ = cancel.cancelled() => break,
                    _ = io.invalidated() => { cancel.cancel(); break; },
                    _ = reconcile(io, &mut scopes) => {},
                }
            },
            received = events.recv() => {
                let Some(event) = received else { break };
                if let Some(c) = candidate(event, &scopes) { buffer.admit(io, c); }
            },
        }
    }
    events.close();
    // Preserve the existing final-flush behavior for fresh input only. Do not
    // burn retry attempts or retry exhausted ciphertext during teardown.
    while !buffer.fresh.is_empty() {
        let n = buffer.fresh.len().min(FLUSH_BATCH_SIZE);
        let batch: Vec<_> = buffer.fresh.drain(..n).collect();
        let size = batch.iter().map(bytes).sum();
        let outcome = io.archive_retry(batch).await;
        buffer.complete(io, n, size, 0, outcome);
    }
    for _ in 0..256 {
        let Ok(event) = events.try_recv() else { break };
        if let Some(c) = candidate(event, &scopes) {
            buffer.admit(io, c);
        }
    }
    buffer.finish()
}
fn candidate(
    event: MatchedEvent,
    scopes: &HashMap<String, MatchedScope>,
) -> Option<ArchiveCandidate> {
    Some(ArchiveCandidate {
        raw_event_json: event.event.as_json(),
        matched_scope: scopes.get(&event.subscription_id)?.clone(),
    })
}

#[cfg(test)]
#[path = "sync_runner_retry_tests.rs"]
mod retry_tests;

async fn reconcile<I: ArchiveSyncIo + ?Sized>(io: &I, scopes: &mut HashMap<String, MatchedScope>) {
    let subscriptions = match io.list_subscriptions().await {
        Ok(subscriptions) => subscriptions,
        Err(error) => {
            eprintln!("buzz-desktop: archive sync: list_save_subscriptions failed: {error}");
            return;
        }
    };
    let (planned, next_scopes) = plan_subscriptions(&subscriptions);
    io.set_subscriptions(planned).await;
    *scopes = next_scopes;
}
