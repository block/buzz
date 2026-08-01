#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Cross-process publication fencing for managed Buzz agent turns.
//!
//! A harness owns a fence file for each ACP process. A publishing subprocess
//! captures the active generation when its command starts, then reacquires a
//! shared lease immediately before submitting the event. Terminal transitions
//! take an exclusive lock, so a completed transition rejects every later lease.

use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

/// Environment variable containing the managed turn's publication fence path.
pub const PUBLICATION_FENCE_ENV: &str = "BUZZ_ACP_PUBLICATION_FENCE";

/// Hidden CLI argument used by the harness to verify fence-capable tooling.
pub const PUBLICATION_FENCE_CAPABILITY_ARG: &str = "__publication-fence-capability";

/// Exact response emitted by a fence-capable Buzz CLI.
pub const PUBLICATION_FENCE_CAPABILITY_RESPONSE: &str = "buzz-publication-fence-v1";

/// Errors returned by publication fence operations.
#[derive(Debug, Error)]
pub enum FenceError {
    /// The fence file could not be opened, locked, read, or written.
    #[error("publication fence I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// The fence file contained malformed state.
    #[error("publication fence state is invalid: {0}")]
    Json(#[from] serde_json::Error),
    /// No active managed turn may publish.
    #[error("managed turn is terminal; publication rejected")]
    Terminal,
    /// The command belongs to an earlier turn generation.
    #[error("managed turn generation changed; stale publication rejected")]
    StaleGeneration,
    /// The publication destination is outside the active turn's scope.
    #[error("publication destination does not match the active managed turn")]
    ScopeMismatch,
    /// The generation counter cannot be advanced safely.
    #[error("publication fence generation exhausted")]
    GenerationExhausted,
    /// An exclusive transition could not drain active publication leases in time.
    #[error("publication fence lease did not drain within {0:?}")]
    LockTimeout(Duration),
}

/// Channel and ordinary reply destination authorized for one managed turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationScope {
    /// Harness turn identifier used for diagnostics.
    pub turn_id: String,
    /// Channel the turn may publish into. `None` allows any channel.
    pub channel_id: Option<Uuid>,
    /// Ordinary reply anchor. A different explicit reply target is rejected.
    pub reply_to: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum FenceStatus {
    Active,
    Terminal,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct FenceState {
    version: u8,
    generation: u64,
    status: FenceStatus,
    turn_id: String,
    channel_id: Option<Uuid>,
    reply_to: Option<String>,
}

/// Harness-owned writer for a single ACP process's publication fence.
#[derive(Clone, Debug)]
pub struct PublicationFence {
    path: PathBuf,
}

impl PublicationFence {
    /// Create a terminal fence at `path`.
    pub fn create(path: impl AsRef<Path>) -> Result<Self, FenceError> {
        let path = path.as_ref().to_path_buf();
        let fence = Self { path };
        let file = open_fence(&fence.path, true)?;
        FileExt::lock_exclusive(&file)?;
        write_state(
            &file,
            &FenceState {
                version: 1,
                generation: 0,
                status: FenceStatus::Terminal,
                turn_id: String::new(),
                channel_id: None,
                reply_to: None,
            },
        )?;
        FileExt::unlock(&file)?;
        Ok(fence)
    }

    /// Return the backing fence path for child-process environment injection.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Remove the fence file after the owning ACP process has been reaped.
    pub fn remove(&self) -> Result<(), FenceError> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Open a new active generation and return its generation number.
    pub fn begin(&self, scope: PublicationScope) -> Result<u64, FenceError> {
        let file = open_fence(&self.path, false)?;
        FileExt::lock_exclusive(&file)?;
        begin_locked(&file, scope)
    }

    /// Open a new active generation after waiting at most `timeout` for leases.
    pub fn begin_with_timeout(
        &self,
        scope: PublicationScope,
        timeout: Duration,
    ) -> Result<u64, FenceError> {
        let file = open_fence(&self.path, false)?;
        lock_exclusive_with_timeout(&file, timeout)?;
        begin_locked(&file, scope)
    }

    /// Mark `generation` terminal. Returns false when a newer generation won.
    pub fn terminate(&self, generation: u64) -> Result<bool, FenceError> {
        let file = open_fence(&self.path, false)?;
        FileExt::lock_exclusive(&file)?;
        terminate_locked(&file, generation)
    }

    /// Mark `generation` terminal after waiting at most `timeout` for leases.
    pub fn terminate_with_timeout(
        &self,
        generation: u64,
        timeout: Duration,
    ) -> Result<bool, FenceError> {
        let file = open_fence(&self.path, false)?;
        lock_exclusive_with_timeout(&file, timeout)?;
        terminate_locked(&file, generation)
    }
}

/// A publication attempt captured while a managed turn was active.
#[derive(Debug)]
pub struct PublicationAttempt {
    path: PathBuf,
    generation: u64,
    channel_id: Uuid,
    reply_to: Option<String>,
}

impl PublicationAttempt {
    /// Capture from [`PUBLICATION_FENCE_ENV`] when managed fencing is enabled.
    ///
    /// Standalone and human-invoked CLI processes normally have no fence
    /// variable and therefore return `Ok(None)` without changing behavior.
    pub fn capture_from_env(
        channel_id: Uuid,
        reply_to: Option<&str>,
    ) -> Result<Option<Self>, FenceError> {
        let Some(path) = std::env::var_os(PUBLICATION_FENCE_ENV) else {
            return Ok(None);
        };
        Self::capture(PathBuf::from(path), channel_id, reply_to).map(Some)
    }

    /// Capture the active generation and destination scope at command start.
    pub fn capture(
        path: impl AsRef<Path>,
        channel_id: Uuid,
        reply_to: Option<&str>,
    ) -> Result<Self, FenceError> {
        let path = path.as_ref().to_path_buf();
        let file = open_fence(&path, false)?;
        FileExt::lock_shared(&file)?;
        let state = read_state(&file)?;
        validate_scope(&state, channel_id, reply_to)?;
        FileExt::unlock(&file)?;
        Ok(Self {
            path,
            generation: state.generation,
            channel_id,
            reply_to: reply_to.map(str::to_owned),
        })
    }

    /// Acquire a shared publication lease immediately before network submit.
    pub fn acquire(self) -> Result<PublicationLease, FenceError> {
        let file = open_fence(&self.path, false)?;
        FileExt::lock_shared(&file)?;
        let state = read_state(&file)?;
        if state.generation != self.generation {
            FileExt::unlock(&file)?;
            return Err(FenceError::StaleGeneration);
        }
        if let Err(error) = validate_scope(&state, self.channel_id, self.reply_to.as_deref()) {
            FileExt::unlock(&file)?;
            return Err(error);
        }
        Ok(PublicationLease { file })
    }
}

/// Shared lock held across the final network submission.
#[derive(Debug)]
pub struct PublicationLease {
    file: File,
}

impl Drop for PublicationLease {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn begin_locked(file: &File, scope: PublicationScope) -> Result<u64, FenceError> {
    let current = read_state(file)?;
    let generation = current
        .generation
        .checked_add(1)
        .ok_or(FenceError::GenerationExhausted)?;
    write_state(
        file,
        &FenceState {
            version: 1,
            generation,
            status: FenceStatus::Active,
            turn_id: scope.turn_id,
            channel_id: scope.channel_id,
            reply_to: scope.reply_to,
        },
    )?;
    FileExt::unlock(file)?;
    Ok(generation)
}

fn terminate_locked(file: &File, generation: u64) -> Result<bool, FenceError> {
    let mut state = read_state(file)?;
    if state.generation != generation {
        FileExt::unlock(file)?;
        return Ok(false);
    }
    if state.status != FenceStatus::Terminal {
        state.status = FenceStatus::Terminal;
        write_state(file, &state)?;
    }
    FileExt::unlock(file)?;
    Ok(true)
}

fn lock_exclusive_with_timeout(file: &File, timeout: Duration) -> Result<(), FenceError> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(Instant::now);
    loop {
        match FileExt::try_lock_exclusive(file) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(FenceError::LockTimeout(timeout));
                }
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(now)
                        .min(Duration::from_millis(10)),
                );
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn validate_scope(
    state: &FenceState,
    channel_id: Uuid,
    reply_to: Option<&str>,
) -> Result<(), FenceError> {
    if state.status != FenceStatus::Active {
        return Err(FenceError::Terminal);
    }
    if state
        .channel_id
        .is_some_and(|expected| expected != channel_id)
    {
        return Err(FenceError::ScopeMismatch);
    }
    if let (Some(expected), Some(actual)) = (state.reply_to.as_deref(), reply_to) {
        if expected != actual {
            return Err(FenceError::ScopeMismatch);
        }
    }
    Ok(())
}

fn open_fence(path: &Path, create: bool) -> Result<File, std::io::Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn read_state(file: &File) -> Result<FenceState, FenceError> {
    let mut reader = file.try_clone()?;
    reader.seek(SeekFrom::Start(0))?;
    Ok(serde_json::from_reader(reader)?)
}

fn write_state(file: &File, state: &FenceState) -> Result<(), FenceError> {
    let mut writer = file.try_clone()?;
    writer.set_len(0)?;
    writer.seek(SeekFrom::Start(0))?;
    serde_json::to_writer(&mut writer, state)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.sync_data()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(channel_id: Uuid, reply_to: &str, turn_id: &str) -> PublicationScope {
        PublicationScope {
            turn_id: turn_id.to_string(),
            channel_id: Some(channel_id),
            reply_to: Some(reply_to.to_string()),
        }
    }

    #[test]
    fn terminal_transition_rejects_attempt_captured_while_active() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("publication-fence.json");
        let fence = PublicationFence::create(&path).expect("create fence");
        let channel_id = Uuid::new_v4();
        let generation = fence
            .begin(scope(channel_id, "root-a", "turn-a"))
            .expect("begin turn");
        let attempt = PublicationAttempt::capture(&path, channel_id, Some("root-a"))
            .expect("capture active attempt");

        assert!(fence.terminate(generation).expect("terminate turn"));
        assert!(matches!(attempt.acquire(), Err(FenceError::Terminal)));
    }

    #[test]
    fn active_matching_generation_receives_publication_lease() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("publication-fence.json");
        let fence = PublicationFence::create(&path).expect("create fence");
        let channel_id = Uuid::new_v4();
        fence
            .begin(scope(channel_id, "root-a", "turn-a"))
            .expect("begin turn");

        let attempt = PublicationAttempt::capture(&path, channel_id, Some("root-a"))
            .expect("capture active attempt");
        let _lease = attempt.acquire().expect("acquire active lease");
    }

    #[test]
    fn newer_generation_rejects_attempt_from_prior_turn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("publication-fence.json");
        let fence = PublicationFence::create(&path).expect("create fence");
        let channel_id = Uuid::new_v4();
        fence
            .begin(scope(channel_id, "root-a", "turn-a"))
            .expect("begin first turn");
        let attempt = PublicationAttempt::capture(&path, channel_id, Some("root-a"))
            .expect("capture first turn");

        fence
            .begin(scope(channel_id, "root-b", "turn-b"))
            .expect("begin second turn");

        assert!(matches!(
            attempt.acquire(),
            Err(FenceError::StaleGeneration)
        ));
    }

    #[test]
    fn capture_rejects_wrong_channel_or_explicit_reply_target() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("publication-fence.json");
        let fence = PublicationFence::create(&path).expect("create fence");
        let channel_id = Uuid::new_v4();
        fence
            .begin(scope(channel_id, "root-a", "turn-a"))
            .expect("begin turn");

        assert!(matches!(
            PublicationAttempt::capture(&path, Uuid::new_v4(), Some("root-a")),
            Err(FenceError::ScopeMismatch)
        ));
        assert!(matches!(
            PublicationAttempt::capture(&path, channel_id, Some("root-b")),
            Err(FenceError::ScopeMismatch)
        ));
    }

    #[test]
    fn terminal_transition_waits_for_in_flight_publication_lease() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("publication-fence.json");
        let fence = PublicationFence::create(&path).expect("create fence");
        let channel_id = Uuid::new_v4();
        let generation = fence
            .begin(scope(channel_id, "root-a", "turn-a"))
            .expect("begin turn");
        let lease = PublicationAttempt::capture(&path, channel_id, Some("root-a"))
            .expect("capture active attempt")
            .acquire()
            .expect("acquire publication lease");
        let (tx, rx) = std::sync::mpsc::channel();
        let closer = fence.clone();
        let thread = std::thread::spawn(move || {
            tx.send(closer.terminate(generation))
                .expect("send close result");
        });

        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(matches!(
            rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        drop(lease);
        assert!(rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("terminal transition completes")
            .expect("terminal transition succeeds"));
        thread.join().expect("join closer");
    }

    #[test]
    fn bounded_terminal_transition_times_out_without_blocking_forever() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("publication-fence.json");
        let fence = PublicationFence::create(&path).expect("create fence");
        let channel_id = Uuid::new_v4();
        let generation = fence
            .begin(scope(channel_id, "root-a", "turn-a"))
            .expect("begin turn");
        let _lease = PublicationAttempt::capture(&path, channel_id, Some("root-a"))
            .expect("capture active attempt")
            .acquire()
            .expect("acquire publication lease");
        let timeout = Duration::from_millis(30);
        let started = Instant::now();

        assert!(matches!(
            fence.terminate_with_timeout(generation, timeout),
            Err(FenceError::LockTimeout(actual)) if actual == timeout
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn stale_termination_cannot_close_new_generation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("publication-fence.json");
        let fence = PublicationFence::create(&path).expect("create fence");
        let channel_id = Uuid::new_v4();
        let old_generation = fence
            .begin(scope(channel_id, "root-a", "turn-a"))
            .expect("begin first turn");
        fence
            .begin(scope(channel_id, "root-b", "turn-b"))
            .expect("begin second turn");

        assert!(!fence
            .terminate(old_generation)
            .expect("ignore stale termination"));
        PublicationAttempt::capture(&path, channel_id, Some("root-b"))
            .expect("new turn remains active");
    }
}
