use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rodio::{mixer::Mixer, Player, Source};

/// Serializes every operation on the TTS player and owns the utterance-boundary
/// bookkeeping that must change atomically when playback is replaced.
///
/// Poison recovery is sound because `PlaybackState` has no partially-valid
/// representation: `Player` replacement is a single assignment, booleans are
/// independently valid at either value, and no mutable reference to the state
/// leaves the locked operation that created it.
pub(super) struct PlaybackCoordinator {
    mixer: Mixer,
    state: Mutex<PlaybackState>,
}

struct PlaybackState {
    player: Player,
    first_append: bool,
    synthesis_in_flight: bool,
    synthesis_generation: u64,
}

pub(super) struct SynthesisFlightGuard {
    playback: Arc<PlaybackCoordinator>,
    generation: u64,
}

impl Drop for SynthesisFlightGuard {
    fn drop(&mut self) {
        let mut state = self.playback.lock();
        if state.synthesis_generation == self.generation {
            state.synthesis_in_flight = false;
        }
    }
}

impl PlaybackCoordinator {
    pub(super) fn new(mixer: &Mixer) -> Self {
        Self {
            mixer: mixer.clone(),
            state: Mutex::new(PlaybackState {
                player: Player::connect_new(mixer),
                first_append: true,
                synthesis_in_flight: false,
                synthesis_generation: 0,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, PlaybackState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn append_if<S>(&self, source: S, authorize: impl FnOnce(bool) -> bool) -> bool
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let mut state = self.lock();
        if !authorize(state.player.empty()) {
            return false;
        }
        state.player.append(source);
        state.first_append = false;
        true
    }

    pub(super) fn append_untracked<S>(&self, source: S)
    where
        S: Source<Item = f32> + Send + 'static,
    {
        self.lock().player.append(source);
    }

    pub(super) fn empty(&self) -> bool {
        self.lock().player.empty()
    }

    pub(super) fn prepare_audio<R>(&self, prepare: impl FnOnce(&mut bool, bool) -> R) -> R {
        let mut state = self.lock();
        let empty = state.player.empty();
        prepare(&mut state.first_append, empty)
    }

    pub(super) fn release_if_drained(&self, release: impl FnOnce()) -> bool {
        let mut state = self.lock();
        if !state.player.empty() || state.first_append {
            return false;
        }
        release();
        state.first_append = true;
        true
    }

    pub(super) fn begin_synthesis(self: &Arc<Self>) -> SynthesisFlightGuard {
        let generation = {
            let mut state = self.lock();
            state.synthesis_generation = state.synthesis_generation.wrapping_add(1);
            state.synthesis_in_flight = true;
            state.synthesis_generation
        };
        SynthesisFlightGuard {
            playback: Arc::clone(self),
            generation,
        }
    }

    pub(super) fn with_playback_live<R>(&self, observe: impl FnOnce(bool) -> R) -> R {
        let state = self.lock();
        observe(!state.player.empty() || state.synthesis_in_flight)
    }

    /// Replace live playback with a fresh queue. The old player is dropped
    /// after releasing the coordinator, so rodio's teardown cannot extend the
    /// critical section. Concurrent cancel observers elect exactly one
    /// replacement because replacement resets both liveness signals.
    pub(super) fn cancel_if_live(&self, authorize: impl FnOnce() -> bool) -> bool {
        let old_player = {
            let mut state = self.lock();
            if (state.player.empty() && !state.synthesis_in_flight) || !authorize() {
                return false;
            }
            state.first_append = true;
            state.synthesis_in_flight = false;
            state.synthesis_generation = state.synthesis_generation.wrapping_add(1);
            std::mem::replace(&mut state.player, Player::connect_new(&self.mixer))
        };
        drop(old_player);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZero,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Barrier,
        },
        thread,
        time::{Duration, Instant},
    };

    use rodio::buffer::SamplesBuffer;

    use super::*;

    fn coordinator() -> (Arc<PlaybackCoordinator>, rodio::mixer::MixerSource) {
        let channels = NonZero::new(1).expect("nonzero channels");
        let rate = NonZero::new(24_000).expect("nonzero rate");
        let (mixer, source) = rodio::mixer::mixer(channels, rate);
        (Arc::new(PlaybackCoordinator::new(&mixer)), source)
    }

    fn append_second(playback: &PlaybackCoordinator) {
        playback.append_if(
            SamplesBuffer::new(
                NonZero::new(1).expect("nonzero channels"),
                NonZero::new(24_000).expect("nonzero rate"),
                vec![0.25; 24_000],
            ),
            |_| true,
        );
    }

    #[test]
    fn cancel_replaces_playback_without_waiting_for_the_mixer() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);

        let started = Instant::now();
        assert!(playback.cancel_if_live(|| true));

        assert!(started.elapsed() < Duration::from_millis(50));
        assert!(playback.empty());
    }

    #[test]
    fn concurrent_cancel_observers_elect_exactly_one_replacement() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        let barrier = Arc::new(Barrier::new(3));
        let replacements = Arc::new(AtomicUsize::new(0));
        let mut threads = Vec::new();
        for _ in 0..2 {
            let playback = Arc::clone(&playback);
            let barrier = Arc::clone(&barrier);
            let replacements = Arc::clone(&replacements);
            threads.push(thread::spawn(move || {
                barrier.wait();
                if playback.cancel_if_live(|| true) {
                    replacements.fetch_add(1, Ordering::Relaxed);
                }
            }));
        }
        barrier.wait();
        for thread in threads {
            thread.join().expect("cancel observer");
        }

        assert_eq!(replacements.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn append_and_cancel_are_one_serialized_public_operation() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        let append_authorized = Arc::new(Barrier::new(2));
        let release_append = Arc::new(Barrier::new(2));
        let append_thread = {
            let playback = Arc::clone(&playback);
            let append_authorized = Arc::clone(&append_authorized);
            let release_append = Arc::clone(&release_append);
            thread::spawn(move || {
                playback.append_if(
                    SamplesBuffer::new(
                        NonZero::new(1).expect("nonzero channels"),
                        NonZero::new(24_000).expect("nonzero rate"),
                        vec![0.5; 24_000],
                    ),
                    |_| {
                        append_authorized.wait();
                        release_append.wait();
                        true
                    },
                )
            })
        };
        append_authorized.wait();
        let cancel_thread = {
            let playback = Arc::clone(&playback);
            thread::spawn(move || playback.cancel_if_live(|| true))
        };
        release_append.wait();

        assert!(append_thread.join().expect("append"));
        assert!(cancel_thread.join().expect("cancel"));
        assert!(
            playback.empty(),
            "cancel must replace the queue after append"
        );
    }

    #[test]
    fn cancellation_rearms_first_append_and_releases_activity_once() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        assert!(playback.cancel_if_live(|| true));
        assert!(!playback.release_if_drained(|| panic!("fresh replacement is not a drain")));
        playback.prepare_audio(|first_append, starts_playback_chunk| {
            assert!(*first_append, "replacement must rearm the first append");
            assert!(
                starts_playback_chunk,
                "the first append after replacement must carry the onset cushion"
            );
        });

        append_second(&playback);
        assert!(!playback.release_if_drained(|| panic!("queued audio is not drained")));
    }
}
