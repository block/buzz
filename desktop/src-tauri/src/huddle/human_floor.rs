//! Shared human-floor handle backed by the TTS playback coordinator.

use std::sync::Arc;

use super::tts_playback::{HumanFloorAuthorization, PlaybackCoordinator};

#[derive(Clone)]
pub(crate) struct HumanFloor {
    playback: Arc<PlaybackCoordinator>,
}

impl std::fmt::Debug for HumanFloor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("HumanFloor").finish_non_exhaustive()
    }
}

impl Default for HumanFloor {
    fn default() -> Self {
        Self::new()
    }
}

impl HumanFloor {
    pub(crate) fn new() -> Self {
        Self {
            playback: Arc::new(PlaybackCoordinator::unbound()),
        }
    }

    pub(super) fn playback(&self) -> Arc<PlaybackCoordinator> {
        Arc::clone(&self.playback)
    }

    #[cfg(test)]
    pub(crate) fn is_blocked(&self) -> bool {
        self.playback.human_floor_blocked()
    }

    pub(crate) fn epoch(&self) -> u64 {
        self.playback.human_floor_epoch()
    }

    pub(super) fn authorization(&self, epoch: u64) -> HumanFloorAuthorization {
        self.playback.human_floor_authorization(epoch)
    }

    #[cfg(test)]
    pub(crate) fn permits(&self, epoch: u64) -> bool {
        self.authorization(epoch) == HumanFloorAuthorization::Permitted
    }

    pub(crate) fn enter_local(&self, route_isolated: bool, sustained_coupled_speech: bool) -> bool {
        self.playback
            .enter_local_human_floor(route_isolated, sustained_coupled_speech)
    }

    pub(crate) fn leave_local(&self) {
        self.playback.leave_local_human_floor();
    }

    pub(crate) fn enter_remote(&self, peer: u8) {
        self.playback.enter_remote_human_floor(peer);
    }

    pub(crate) fn leave_remote(&self, peer: u8) {
        self.playback.leave_remote_human_floor(peer);
    }

    pub(crate) fn clear_remote(&self) {
        self.playback.clear_remote_human_floor();
    }
}
