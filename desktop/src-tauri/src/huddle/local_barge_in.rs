//! Local VAD barge-in policy and coupled-output debounce.

use super::human_floor::HumanFloor;

/// Whether local audio should use VAD barge-in for this frame.
///
/// This currently matches STT's `vad_flush_allowed`, but the two decisions are
/// kept separate deliberately: one assigns cancellation ownership and the
/// other controls utterance endpointing.
pub(super) fn enabled(ptt_mode: bool, manually_open: bool, ptt_held: bool) -> bool {
    !ptt_mode || (manually_open && !ptt_held)
}

/// Consecutive 16 ms VAD-positive frames required to restore local barge-in
/// on acoustically coupled output. The prior implementation shipped 20 frames
/// after 5 frames caused speaker-bleed self-cancellation (`b29c8cdaa^`).
const COUPLED_BARGE_IN_FRAMES: usize = 20;

#[derive(Debug, Default)]
pub(super) struct LocalBargeIn {
    acquired_floor: bool,
    coupled_positive_frames: usize,
}

impl LocalBargeIn {
    pub(super) fn observe(
        &mut self,
        probability: f32,
        confirmed_onset: bool,
        human_floor: &HumanFloor,
        output_device: Option<&str>,
        onset_threshold: f32,
    ) {
        if self.acquired_floor {
            return;
        }
        let sustained_coupled = self.track_sustained_coupled(probability, onset_threshold);
        if !confirmed_onset && !sustained_coupled {
            return;
        }
        let route_isolated = super::audio_output::output_route_is_isolated(output_device);
        self.acquire(human_floor, route_isolated, sustained_coupled);
    }

    pub(super) fn acquire(
        &mut self,
        human_floor: &HumanFloor,
        route_isolated: bool,
        sustained_coupled: bool,
    ) {
        self.acquired_floor = human_floor.enter_local(route_isolated, sustained_coupled);
    }

    fn track_sustained_coupled(&mut self, probability: f32, onset_threshold: f32) -> bool {
        if probability > onset_threshold {
            self.coupled_positive_frames = self.coupled_positive_frames.saturating_add(1);
        } else {
            self.coupled_positive_frames = 0;
        }
        self.coupled_positive_frames >= COUPLED_BARGE_IN_FRAMES
    }

    pub(super) fn release(&mut self, human_floor: &HumanFloor) {
        if self.acquired_floor {
            human_floor.leave_local();
        }
        *self = Self::default();
    }
}

#[derive(Debug)]
pub(super) struct WorkerLocalBargeIn {
    state: LocalBargeIn,
    human_floor: HumanFloor,
}

impl WorkerLocalBargeIn {
    pub(super) fn new(human_floor: HumanFloor) -> Self {
        Self {
            state: LocalBargeIn::default(),
            human_floor,
        }
    }
}

impl std::ops::Deref for WorkerLocalBargeIn {
    type Target = LocalBargeIn;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl std::ops::DerefMut for WorkerLocalBargeIn {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Drop for WorkerLocalBargeIn {
    fn drop(&mut self) {
        self.state.release(&self.human_floor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_open_mic_enables_vad_barge_in_in_ptt_mode() {
        assert!(enabled(true, true, false));
        assert!(!enabled(true, false, false));
        assert!(!enabled(true, true, true));
        assert!(enabled(false, false, false));
    }

    #[test]
    fn manual_open_ptt_sustained_speech_acquires_coupled_floor() {
        assert!(enabled(true, true, false));
        let human_floor = HumanFloor::new();
        let mut barge_in = LocalBargeIn::default();
        for _ in 0..COUPLED_BARGE_IN_FRAMES {
            let sustained = barge_in.track_sustained_coupled(0.9, 0.5);
            if sustained {
                barge_in.acquire(&human_floor, false, true);
            }
        }
        assert!(barge_in.acquired_floor);
        assert!(human_floor.is_blocked());
    }

    #[test]
    fn coupled_barge_in_requires_twenty_consecutive_positive_frames() {
        let mut barge_in = LocalBargeIn::default();
        for _ in 0..COUPLED_BARGE_IN_FRAMES - 1 {
            assert!(!barge_in.track_sustained_coupled(0.9, 0.5));
        }
        assert!(barge_in.track_sustained_coupled(0.9, 0.5));
    }

    #[test]
    fn coupled_barge_in_debounce_resets_on_a_non_speech_frame() {
        let mut barge_in = LocalBargeIn::default();
        for _ in 0..COUPLED_BARGE_IN_FRAMES - 1 {
            assert!(!barge_in.track_sustained_coupled(0.9, 0.5));
        }
        assert!(!barge_in.track_sustained_coupled(0.1, 0.5));
        assert!(!barge_in.track_sustained_coupled(0.9, 0.5));
    }
}
