use std::{
    collections::HashSet,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use rodio::{mixer::Mixer, Player, Source};

/// Conservative guard after rodio reports drained. Max measured about 12 ms
/// of cancellation tail on current-main CoreAudio and about 1 ms after player
/// replacement; 100 ms safely bounds those observed paths while the phase-1
/// route matrix determines whether this can be narrowed.
const OUTPUT_TAIL_HANGOVER: Duration = Duration::from_millis(100);

/// Serializes every operation on the TTS player and owns the utterance-boundary
/// bookkeeping that must change atomically when playback is replaced.
///
/// Poison recovery is sound because `PlaybackState` has no partially-valid
/// representation: `Player` replacement is a single assignment, booleans are
/// independently valid at either value, and no mutable reference to the state
/// leaves the locked operation that created it.
pub(super) struct PlaybackCoordinator {
    mixer: Mutex<Option<Mixer>>,
    state: Mutex<PlaybackState>,
}

struct PlaybackState {
    player: Option<Player>,
    /// `true` while no append has been committed since the last utterance
    /// boundary. Only `append_if` clears it, so it records appends that were
    /// actually queued — never one the authorization refused.
    first_append: bool,
    synthesis_in_flight: bool,
    synthesis_generation: u64,
    output_lease: OutputLease,
    human_floor: HumanFloorState,
}

#[derive(Default)]
enum OutputLease {
    #[default]
    Inactive,
    Active,
    HangoverUntil(Instant),
}

impl OutputLease {
    fn is_live_at(&mut self, now: Instant) -> bool {
        match self {
            Self::Inactive => false,
            Self::Active => true,
            Self::HangoverUntil(deadline) if now < *deadline => true,
            Self::HangoverUntil(_) => {
                *self = Self::Inactive;
                false
            }
        }
    }

    fn begin_hangover(&mut self, now: Instant) {
        if !matches!(self, Self::Inactive) {
            *self = Self::HangoverUntil(now + OUTPUT_TAIL_HANGOVER);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HumanFloorAuthorization {
    Permitted,
    Blocked,
    Stale,
}

#[derive(Default)]
struct HumanFloorState {
    epoch: u64,
    local: bool,
    remote: HashSet<u8>,
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
    #[cfg(test)]
    pub(super) fn new(mixer: &Mixer) -> Self {
        let coordinator = Self::unbound();
        coordinator.bind_mixer(mixer);
        coordinator
    }

    pub(super) fn unbound() -> Self {
        Self {
            mixer: Mutex::new(None),
            state: Mutex::new(PlaybackState {
                player: None,
                first_append: true,
                synthesis_in_flight: false,
                synthesis_generation: 0,
                output_lease: OutputLease::Inactive,
                human_floor: HumanFloorState::default(),
            }),
        }
    }

    pub(super) fn bind_mixer(&self, mixer: &Mixer) {
        *self.mixer.lock().unwrap_or_else(PoisonError::into_inner) = Some(mixer.clone());
        let mut state = self.lock();
        if state.player.is_none() {
            state.player = Some(Player::connect_new(mixer));
        }
    }

    fn lock(&self) -> MutexGuard<'_, PlaybackState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Queue `source` when `authorize` accepts, then publish the append with
    /// `commit` before releasing the coordinator. `commit` runs under the lock
    /// so an append and the activity state it implies are one transition: a
    /// concurrent cancellation either replaces the queue before this append is
    /// authorized, or observes the committed state after it — never lands its
    /// own release between the two and gets overwritten.
    #[cfg(test)]
    pub(super) fn append_if<S>(
        &self,
        source: S,
        authorize: impl FnOnce(bool) -> bool,
        commit: impl FnOnce(),
    ) -> bool
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let mut state = self.lock();
        if !authorize(state.player.as_ref().is_none_or(Player::empty)) {
            return false;
        }
        let Some(player) = state.player.as_ref() else {
            return false;
        };
        player.append(source);
        state.first_append = false;
        state.output_lease = OutputLease::Active;
        commit();
        true
    }

    pub(super) fn append_untracked<S>(&self, source: S)
    where
        S: Source<Item = f32> + Send + 'static,
    {
        if let Some(player) = self.lock().player.as_ref() {
            player.append(source);
        }
    }

    pub(super) fn empty(&self) -> bool {
        self.lock().player.as_ref().is_none_or(Player::empty)
    }

    /// Observe playback emptiness under the coordinator so the onset decision
    /// for the audio being built is serialized with append and cancellation.
    pub(super) fn prepare_audio<R>(&self, prepare: impl FnOnce(bool) -> R) -> R {
        let state = self.lock();
        let empty = state.player.as_ref().is_none_or(Player::empty);
        prepare(empty)
    }

    pub(super) fn release_if_drained(&self, release: impl FnOnce()) -> bool {
        let mut state = self.lock();
        if !state.player.as_ref().is_none_or(Player::empty) || state.first_append {
            return false;
        }
        release();
        state.first_append = true;
        state.output_lease.begin_hangover(Instant::now());
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
        observe(!state.player.as_ref().is_none_or(Player::empty) || state.synthesis_in_flight)
    }

    /// Replace live playback with a fresh queue, publishing the replacement
    /// with `commit` before releasing the coordinator. The old player is
    /// dropped after releasing, so rodio's teardown cannot extend the critical
    /// section. Concurrent cancel observers elect exactly one replacement
    /// because replacement resets both liveness signals.
    ///
    /// `commit` is the mirror of `append_if`'s: a replacement and the activity
    /// state it implies are one transition, so an append that wins the lock
    /// handoff after this cancellation cannot have its own publication
    /// overwritten by a `false` landing late.
    pub(super) fn cancel_if_live(
        &self,
        authorize: impl FnOnce() -> bool,
        commit: impl FnOnce(),
    ) -> bool {
        let replacement = self
            .mixer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Player::connect_new);
        let old_player = {
            let mut state = self.lock();
            if (state.player.as_ref().is_none_or(Player::empty) && !state.synthesis_in_flight)
                || !authorize()
            {
                return false;
            }
            state.first_append = true;
            state.synthesis_in_flight = false;
            state.synthesis_generation = state.synthesis_generation.wrapping_add(1);
            state.output_lease.begin_hangover(Instant::now());
            let old_player = std::mem::replace(&mut state.player, replacement);
            commit();
            old_player
        };
        drop(old_player);
        true
    }

    #[cfg(test)]
    pub(super) fn human_floor_blocked(&self) -> bool {
        let state = self.lock();
        state.human_floor.local || !state.human_floor.remote.is_empty()
    }

    pub(super) fn human_floor_epoch(&self) -> u64 {
        self.lock().human_floor.epoch
    }

    pub(super) fn human_floor_authorization(&self, epoch: u64) -> HumanFloorAuthorization {
        Self::human_floor_authorization_locked(&self.lock(), epoch)
    }

    fn human_floor_authorization_locked(
        state: &PlaybackState,
        epoch: u64,
    ) -> HumanFloorAuthorization {
        if state.human_floor.local || !state.human_floor.remote.is_empty() {
            HumanFloorAuthorization::Blocked
        } else if state.human_floor.epoch != epoch {
            HumanFloorAuthorization::Stale
        } else {
            HumanFloorAuthorization::Permitted
        }
    }

    #[cfg(test)]
    pub(super) fn human_floor_permits(&self, epoch: u64) -> bool {
        self.human_floor_authorization(epoch) == HumanFloorAuthorization::Permitted
    }

    pub(super) fn append_if_human_floor_permits<S>(
        &self,
        source: S,
        epoch: u64,
        authorize: impl FnOnce(bool) -> bool,
        commit: impl FnOnce(),
    ) -> HumanFloorAuthorization
    where
        S: Source<Item = f32> + Send + 'static,
    {
        let mut state = self.lock();
        let floor_authorization = Self::human_floor_authorization_locked(&state, epoch);
        if floor_authorization != HumanFloorAuthorization::Permitted {
            return floor_authorization;
        }
        if !authorize(state.player.as_ref().is_none_or(Player::empty)) {
            return HumanFloorAuthorization::Stale;
        }
        let Some(player) = state.player.as_ref() else {
            return HumanFloorAuthorization::Stale;
        };
        player.append(source);
        state.first_append = false;
        state.output_lease = OutputLease::Active;
        commit();
        HumanFloorAuthorization::Permitted
    }

    pub(super) fn enter_local_human_floor(
        &self,
        route_isolated: bool,
        sustained_coupled_speech: bool,
    ) -> bool {
        let replacement = self
            .mixer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Player::connect_new);
        let old_player = {
            let mut state = self.lock();
            let output_live =
                state.synthesis_in_flight || state.output_lease.is_live_at(Instant::now());
            if state.human_floor.local
                || (output_live && !route_isolated && !sustained_coupled_speech)
            {
                return false;
            }
            state.human_floor.local = true;
            Self::commit_human_floor_onset(&mut state, replacement)
        };
        drop(old_player);
        true
    }

    pub(super) fn leave_local_human_floor(&self) {
        self.lock().human_floor.local = false;
    }

    pub(super) fn enter_remote_human_floor(&self, peer: u8) {
        self.enter_human_floor(|floor| floor.remote.insert(peer));
    }

    pub(super) fn leave_remote_human_floor(&self, peer: u8) {
        self.lock().human_floor.remote.remove(&peer);
    }

    pub(super) fn clear_remote_human_floor(&self) {
        self.lock().human_floor.remote.clear();
    }

    fn enter_human_floor(&self, enter: impl FnOnce(&mut HumanFloorState) -> bool) {
        let replacement = self
            .mixer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Player::connect_new);
        let old_player = {
            let mut state = self.lock();
            if !enter(&mut state.human_floor) {
                return;
            }
            Self::commit_human_floor_onset(&mut state, replacement)
        };
        drop(old_player);
    }

    fn commit_human_floor_onset(
        state: &mut PlaybackState,
        replacement: Option<Player>,
    ) -> Option<Player> {
        state.human_floor.epoch = state.human_floor.epoch.wrapping_add(1);
        state.first_append = true;
        state.synthesis_in_flight = false;
        state.synthesis_generation = state.synthesis_generation.wrapping_add(1);
        state.output_lease.begin_hangover(Instant::now());
        std::mem::replace(&mut state.player, replacement)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        num::NonZero,
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
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
            || {},
        );
    }

    fn one_second_source() -> SamplesBuffer {
        SamplesBuffer::new(
            NonZero::new(1).expect("nonzero channels"),
            NonZero::new(24_000).expect("nonzero rate"),
            vec![0.25; 24_000],
        )
    }

    #[test]
    fn floor_authorized_append_does_not_reenter_the_coordinator_lock() {
        let (playback, _unpulled_source) = coordinator();
        let epoch = playback.human_floor_epoch();
        let (completed_tx, completed_rx) = std::sync::mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let authorization =
                playback.append_if_human_floor_permits(one_second_source(), epoch, |_| true, || {});
            completed_tx
                .send(authorization)
                .expect("completion receiver");
        });

        assert_eq!(
            completed_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("floor-authorized append must not deadlock"),
            HumanFloorAuthorization::Permitted
        );
        worker.join().expect("append worker");
    }

    #[test]
    fn text_queued_during_a_held_floor_is_permitted_after_release() {
        let (playback, _unpulled_source) = coordinator();
        assert!(playback.enter_local_human_floor(true, false));
        let queued_epoch = playback.human_floor_epoch();

        assert_eq!(
            playback.human_floor_authorization(queued_epoch),
            HumanFloorAuthorization::Blocked
        );
        playback.leave_local_human_floor();
        assert_eq!(
            playback.human_floor_authorization(queued_epoch),
            HumanFloorAuthorization::Permitted
        );
        assert_eq!(
            playback.append_if_human_floor_permits(
                one_second_source(),
                queued_epoch,
                |_| true,
                || {},
            ),
            HumanFloorAuthorization::Permitted
        );
    }

    #[test]
    fn human_onset_replaces_playback_and_invalidates_late_append() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        let stale_epoch = playback.human_floor_epoch();

        assert!(playback.enter_local_human_floor(true, false));

        assert!(playback.empty());
        assert!(playback.human_floor_blocked());
        assert!(!playback.human_floor_permits(stale_epoch));
        playback.leave_local_human_floor();
        assert!(!playback.human_floor_blocked());
        assert!(!playback.human_floor_permits(stale_epoch));
    }

    #[test]
    fn coupled_local_onset_while_idle_blocks_delayed_tts() {
        let (playback, _unpulled_source) = coordinator();
        let delayed_tts_epoch = playback.human_floor_epoch();

        assert!(playback.enter_local_human_floor(false, false));

        assert!(playback.human_floor_blocked());
        assert!(!playback.human_floor_permits(delayed_tts_epoch));
    }

    #[test]
    fn coupled_local_onset_during_output_is_rejected_as_ambiguous_echo() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        let epoch = playback.human_floor_epoch();

        assert!(!playback.enter_local_human_floor(false, false));

        assert!(!playback.human_floor_blocked());
        assert!(playback.human_floor_permits(epoch));
        assert!(!playback.empty());
    }

    #[test]
    fn sustained_coupled_speech_overrides_live_output_suppression() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        let stale_epoch = playback.human_floor_epoch();

        assert!(playback.enter_local_human_floor(false, true));

        assert!(playback.empty());
        assert!(playback.human_floor_blocked());
        assert!(!playback.human_floor_permits(stale_epoch));
    }

    #[test]
    fn coupled_local_onset_during_output_tail_hangover_is_rejected() {
        let (playback, mut source) = coordinator();
        append_second(&playback);
        while !playback.empty() {
            assert!(
                source.next().is_some(),
                "the mixer source outlives the queue"
            );
        }
        assert!(playback.release_if_drained(|| {}));

        assert!(!playback.enter_local_human_floor(false, false));
        assert!(!playback.human_floor_blocked());
    }

    #[test]
    fn coupled_local_onset_after_output_tail_hangover_is_accepted() {
        let (playback, mut source) = coordinator();
        append_second(&playback);
        while !playback.empty() {
            assert!(
                source.next().is_some(),
                "the mixer source outlives the queue"
            );
        }
        assert!(playback.release_if_drained(|| {}));
        playback.lock().output_lease =
            OutputLease::HangoverUntil(Instant::now() - Duration::from_millis(1));

        assert!(playback.enter_local_human_floor(false, false));
        assert!(playback.human_floor_blocked());
    }

    #[test]
    fn accepted_append_renews_an_expiring_output_lease() {
        let (playback, _unpulled_source) = coordinator();
        playback.lock().output_lease =
            OutputLease::HangoverUntil(Instant::now() + Duration::from_millis(1));

        append_second(&playback);

        assert!(matches!(playback.lock().output_lease, OutputLease::Active));
    }

    #[test]
    fn remote_onset_while_idle_blocks_delayed_tts() {
        let (playback, _unpulled_source) = coordinator();
        let delayed_tts_epoch = playback.human_floor_epoch();

        playback.enter_remote_human_floor(7);

        assert!(playback.human_floor_blocked());
        assert!(!playback.human_floor_permits(delayed_tts_epoch));
    }

    #[test]
    fn local_and_remote_sources_hold_the_same_floor_until_each_releases() {
        let (playback, _unpulled_source) = coordinator();
        assert!(playback.enter_local_human_floor(true, false));
        let local_epoch = playback.human_floor_epoch();
        playback.enter_remote_human_floor(7);
        assert_ne!(playback.human_floor_epoch(), local_epoch);

        playback.leave_local_human_floor();
        assert!(playback.human_floor_blocked());
        playback.leave_remote_human_floor(7);
        assert!(!playback.human_floor_blocked());
    }

    #[test]
    fn cancel_replaces_playback_without_waiting_for_the_mixer() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);

        let started = Instant::now();
        assert!(playback.cancel_if_live(|| true, || {}));

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
                if playback.cancel_if_live(|| true, || {}) {
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
                    || {},
                )
            })
        };
        append_authorized.wait();
        let cancel_thread = {
            let playback = Arc::clone(&playback);
            thread::spawn(move || playback.cancel_if_live(|| true, || {}))
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
    fn an_accepted_append_publishes_its_activity_inside_the_append_transition() {
        let (playback, _unpulled_source) = coordinator();
        let committed = Arc::new(AtomicBool::new(false));

        let appended = playback.append_if(
            SamplesBuffer::new(
                NonZero::new(1).expect("nonzero channels"),
                NonZero::new(24_000).expect("nonzero rate"),
                vec![0.25; 24_000],
            ),
            |_| true,
            || {
                // The coordinator is still held, so no cancellation can land a
                // release between queueing this audio and publishing it.
                assert!(
                    playback.state.try_lock().is_err(),
                    "commit must run inside the append's critical section"
                );
                committed.store(true, Ordering::Release);
            },
        );

        assert!(appended);
        assert!(
            committed.load(Ordering::Acquire),
            "an accepted append must publish"
        );
        assert!(
            playback.state.try_lock().is_ok(),
            "the coordinator is released once the append returns"
        );
    }

    #[test]
    fn a_refused_append_publishes_nothing_and_leaves_the_onset_armed() {
        let (playback, _unpulled_source) = coordinator();

        // The worker builds its buffer under the coordinator, then the append
        // is refused — cancelled, or owned by another speaker.
        playback.prepare_audio(|starts_playback_chunk| {
            assert!(starts_playback_chunk, "a fresh coordinator is idle");
        });
        let appended = playback.append_if(
            SamplesBuffer::new(
                NonZero::new(1).expect("nonzero channels"),
                NonZero::new(24_000).expect("nonzero rate"),
                vec![0.25; 24_000],
            ),
            |_| false,
            || panic!("a refused append must publish nothing"),
        );

        assert!(!appended);
        // Nothing was queued, so this is not a drained utterance: releasing
        // here would drop the mic gate and log a drain for audio that never
        // played, and cost the next append its onset cushion.
        assert!(
            !playback.release_if_drained(|| panic!("a refused append is not a drain")),
            "a refused append must not present as a drained utterance"
        );
    }

    #[test]
    fn an_appended_utterance_still_releases_exactly_once_when_it_drains() {
        let (playback, mut source) = coordinator();
        append_second(&playback);

        assert!(
            !playback.release_if_drained(|| panic!("queued audio is not drained")),
            "queued audio must not release"
        );
        while !playback.empty() {
            assert!(
                source.next().is_some(),
                "the mixer source outlives the queue"
            );
        }

        let releases = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let releases = Arc::clone(&releases);
            playback.release_if_drained(move || {
                releases.fetch_add(1, Ordering::Relaxed);
            });
        }

        assert_eq!(
            releases.load(Ordering::Relaxed),
            1,
            "a drained utterance releases once and rearms the onset"
        );
    }

    /// The reverse direction of the same barrier: a cancellation must publish
    /// its release *inside* the replacement. The cancelling thread is
    /// otherwise past its replacement and about to release the mic gate, while
    /// an append that wins the coordinator handoff has already published its
    /// own `true` — a `false` landing outside the replacement would ungate the
    /// mic for audio that is actually playing, and VAD would hear our own TTS
    /// and barge in on it.
    #[test]
    fn a_replacement_publishes_its_release_inside_the_cancel_transition() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        let committed = Arc::new(AtomicBool::new(false));

        let replaced = playback.cancel_if_live(
            || true,
            || {
                // Still held: no append can commit its own activity between
                // this replacement and the release it implies.
                assert!(
                    playback.state.try_lock().is_err(),
                    "commit must run inside the cancellation's critical section"
                );
                committed.store(true, Ordering::Release);
            },
        );

        assert!(replaced, "live playback must be replaced");
        assert!(
            committed.load(Ordering::Acquire),
            "a replacement must publish"
        );
        assert!(
            playback.state.try_lock().is_ok(),
            "the coordinator is released once the cancellation returns"
        );
    }

    /// A cancellation that replaces nothing publishes nothing: the caller
    /// still owns releasing the gate, and a replacement that never happened
    /// must not present as one.
    #[test]
    fn a_cancellation_with_nothing_live_publishes_nothing() {
        let (playback, _unpulled_source) = coordinator();

        assert!(!playback.cancel_if_live(|| true, || panic!("nothing was replaced")));
        assert!(!playback.cancel_if_live(|| false, || panic!("cancellation was refused")));
    }

    /// Both publication directions under real contention: whichever
    /// transition takes the coordinator last decides, and the activity flag
    /// must describe the player that survived.
    #[test]
    fn a_cancellation_and_an_append_never_disagree_about_the_mic_gate() {
        for _ in 0..256 {
            let (playback, _unpulled_source) = coordinator();
            let tts_active = Arc::new(AtomicBool::new(true));
            append_second(&playback);
            let barrier = Arc::new(Barrier::new(2));

            let canceller = {
                let playback = Arc::clone(&playback);
                let tts_active = Arc::clone(&tts_active);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    playback.cancel_if_live(|| true, || tts_active.store(false, Ordering::Release))
                })
            };
            let appender = {
                let playback = Arc::clone(&playback);
                let tts_active = Arc::clone(&tts_active);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    playback.append_if(
                        SamplesBuffer::new(
                            NonZero::new(1).expect("nonzero channels"),
                            NonZero::new(24_000).expect("nonzero rate"),
                            vec![0.5; 24_000],
                        ),
                        |_| true,
                        || tts_active.store(true, Ordering::Release),
                    )
                })
            };

            let replaced = canceller.join().expect("canceller");
            let appended = appender.join().expect("appender");
            assert!(replaced, "live playback must be replaced");
            assert!(appended, "the append is authorized either way");

            assert_eq!(
                tts_active.load(Ordering::Acquire),
                !playback.empty(),
                "the mic gate must agree with the player that survived"
            );
        }
    }

    #[test]
    fn cancellation_rearms_first_append_and_releases_activity_once() {
        let (playback, _unpulled_source) = coordinator();
        append_second(&playback);
        assert!(playback.cancel_if_live(|| true, || {}));
        assert!(!playback.release_if_drained(|| panic!("fresh replacement is not a drain")));
        playback.prepare_audio(|starts_playback_chunk| {
            assert!(
                starts_playback_chunk,
                "the first append after replacement must carry the onset cushion"
            );
        });

        append_second(&playback);
        assert!(!playback.release_if_drained(|| panic!("queued audio is not drained")));
    }
}
