//! Huddle-audio publishing handles for locally synthesized agent speech.
//!
//! The relay socket itself lives in `relay_api`; this module owns the small,
//! synchronous seam the TTS worker needs. Each publisher is authenticated as
//! the agent whose speech it carries, so the existing peer-index roster keeps
//! remote playback attributed to the agent instead of the hosting human.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// One prepared Pocket-TTS buffer, still at the model's native 24 kHz rate.
#[derive(Debug)]
pub(crate) struct TtsBroadcastPacket {
    pub(crate) epoch: u64,
    pub(crate) speaker_generation: u64,
    pub(crate) samples_24k: Vec<f32>,
}

/// `peer_index -> active local publisher count` for sockets publishing Pocket
/// TTS synthesized by this desktop.
pub(crate) type LocalTtsPublishers = Arc<Mutex<HashMap<u8, usize>>>;

/// A live registration for one locally synthesized publisher socket. The lease
/// is owned by the socket task, so receive-side suppression ends immediately
/// when that socket exits even if its command handle has not been replaced yet.
pub(crate) struct LocalTtsPublisherLease {
    peer_index: u8,
    local_publishers: LocalTtsPublishers,
}

impl LocalTtsPublisherLease {
    pub(crate) fn new(peer_index: u8, local_publishers: LocalTtsPublishers) -> Self {
        *local_publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(peer_index)
            .or_default() += 1;
        Self {
            peer_index,
            local_publishers,
        }
    }
}

impl Drop for LocalTtsPublisherLease {
    fn drop(&mut self) {
        let mut local_publishers = self
            .local_publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(count) = local_publishers.get_mut(&self.peer_index) else {
            return;
        };
        *count -= 1;
        if *count == 0 {
            local_publishers.remove(&self.peer_index);
        }
    }
}

/// A live, agent-authenticated audio publisher.
#[derive(Debug)]
pub(crate) struct TtsAudioPublisher {
    tx: mpsc::Sender<TtsBroadcastPacket>,
    cancel: CancellationToken,
    epoch: Arc<AtomicU64>,
    speaker_generation: Arc<AtomicU64>,
}

impl TtsAudioPublisher {
    pub(crate) fn new(tx: mpsc::Sender<TtsBroadcastPacket>, cancel: CancellationToken) -> Self {
        Self {
            tx,
            cancel,
            epoch: Arc::new(AtomicU64::new(0)),
            speaker_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(crate) fn version_state(&self) -> (Arc<AtomicU64>, Arc<AtomicU64>) {
        (
            Arc::clone(&self.epoch),
            Arc::clone(&self.speaker_generation),
        )
    }

    fn set_speaker_generation(&self, generation: u64) {
        self.speaker_generation.store(generation, Ordering::Release);
    }

    fn is_closed(&self) -> bool {
        self.cancel.is_cancelled() || self.tx.is_closed()
    }

    fn publish(&self, speaker_generation: u64, samples_24k: Vec<f32>) {
        if speaker_generation != self.speaker_generation.load(Ordering::Acquire) {
            return;
        }
        let packet = TtsBroadcastPacket {
            epoch: self.epoch.load(Ordering::Acquire),
            speaker_generation,
            samples_24k,
        };
        if let Err(error) = self.tx.try_send(packet) {
            eprintln!(
                "buzz-desktop: tts broadcast status=dropped reason=publisher_backpressure error={error}"
            );
        }
    }

    fn cancel_pending(&self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
    }

    fn shutdown(&self) {
        self.cancel.cancel();
    }
}

/// Thread-safe registry shared by the TTS worker, cancellation monitor, and
/// async command path that establishes publishers before speech is queued.
#[derive(Clone, Debug, Default)]
pub(super) struct TtsBroadcasters {
    publishers: Arc<Mutex<HashMap<String, TtsAudioPublisher>>>,
}

impl TtsBroadcasters {
    pub(super) fn contains(&self, speaker_pubkey: &str) -> bool {
        self.publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&speaker_pubkey.to_ascii_lowercase())
            .is_some_and(|publisher| !publisher.is_closed())
    }

    pub(super) fn register(
        &self,
        speaker_pubkey: &str,
        publisher: TtsAudioPublisher,
        speaker_generation: u64,
    ) {
        publisher.set_speaker_generation(speaker_generation);
        let replaced = self
            .publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(speaker_pubkey.to_ascii_lowercase(), publisher);
        if let Some(replaced) = replaced {
            replaced.shutdown();
        }
    }

    pub(super) fn publish(
        &self,
        speaker_pubkey: &str,
        speaker_generation: u64,
        samples_24k: Vec<f32>,
    ) {
        let publishers = self
            .publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(publisher) = publishers.get(&speaker_pubkey.to_ascii_lowercase()) {
            publisher.publish(speaker_generation, samples_24k);
        }
    }

    pub(super) fn cancel_speaker(&self, speaker_pubkey: &str, speaker_generation: u64) {
        let publishers = self
            .publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(publisher) = publishers.get(&speaker_pubkey.to_ascii_lowercase()) {
            publisher.set_speaker_generation(speaker_generation);
            publisher.cancel_pending();
        }
    }

    pub(super) fn remove_speaker(&self, speaker_pubkey: &str) {
        let removed = self
            .publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&speaker_pubkey.to_ascii_lowercase());
        if let Some(removed) = removed {
            removed.shutdown();
        }
    }

    pub(super) fn cancel_all(&self) {
        for publisher in self
            .publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
        {
            publisher.cancel_pending();
        }
    }

    pub(super) fn shutdown(&self) {
        let mut publishers = self
            .publishers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for publisher in publishers.values() {
            publisher.shutdown();
        }
        publishers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publisher_lifetime_tracks_local_synthesis_without_replacement_gaps() {
        let local_publishers = LocalTtsPublishers::default();
        let first = LocalTtsPublisherLease::new(3, Arc::clone(&local_publishers));
        assert_eq!(
            local_publishers.lock().expect("local publishers").get(&3),
            Some(&1)
        );

        let replacement = LocalTtsPublisherLease::new(3, Arc::clone(&local_publishers));
        drop(first);
        assert_eq!(
            local_publishers.lock().expect("local publishers").get(&3),
            Some(&1),
            "dropping a replaced socket must not expose its live replacement"
        );

        drop(replacement);
        assert!(local_publishers
            .lock()
            .expect("local publishers")
            .is_empty());
    }

    #[test]
    fn cancellation_invalidates_queued_packet_versions() {
        let (tx, mut rx) = mpsc::channel(2);
        let publisher = TtsAudioPublisher::new(tx, CancellationToken::new());
        let (epoch, generation) = publisher.version_state();
        publisher.set_speaker_generation(4);

        publisher.publish(4, vec![0.25]);
        let queued = rx.try_recv().expect("queued audio");
        assert_eq!(queued.epoch, 0);
        assert_eq!(queued.speaker_generation, 4);

        publisher.cancel_pending();
        assert_ne!(queued.epoch, epoch.load(Ordering::Acquire));

        publisher.set_speaker_generation(5);
        publisher.publish(4, vec![0.5]);
        assert!(
            rx.try_recv().is_err(),
            "stale speaker audio must be dropped"
        );
        assert_eq!(generation.load(Ordering::Acquire), 5);
    }
}
