//! Pure decision gate between VAD silence and the existing STT flush path.

#[cfg_attr(not(test), allow(dead_code))] // Constructed by the optional classifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TurnDecision {
    Hold,
    Shift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SilenceAction {
    Flush,
    Keep,
}

pub(super) fn action_after_vad_silence(
    flag_on: bool,
    vad_flush_allowed: bool,
    decision: Option<TurnDecision>,
) -> SilenceAction {
    if !vad_flush_allowed {
        return SilenceAction::Keep;
    }

    match (flag_on, decision) {
        (true, Some(TurnDecision::Hold)) => SilenceAction::Keep,
        _ => SilenceAction::Flush,
    }
}

#[cfg(test)]
mod tests {
    use super::{action_after_vad_silence, SilenceAction, TurnDecision};

    #[test]
    fn g2_1_shift_flushes_after_vad_silence() {
        assert_eq!(
            action_after_vad_silence(true, true, Some(TurnDecision::Shift)),
            SilenceAction::Flush
        );
    }

    #[test]
    fn b2_1_hold_keeps_the_buffer_after_vad_silence() {
        assert_eq!(
            action_after_vad_silence(true, true, Some(TurnDecision::Hold)),
            SilenceAction::Keep
        );
    }

    #[test]
    fn b2_2_ptt_keeps_the_buffer_even_if_the_classifier_shifts() {
        assert_eq!(
            action_after_vad_silence(true, false, Some(TurnDecision::Shift)),
            SilenceAction::Keep
        );
    }

    #[test]
    fn flag_or_model_unavailable_fails_open_to_flush() {
        assert_eq!(
            action_after_vad_silence(false, true, Some(TurnDecision::Hold)),
            SilenceAction::Flush
        );
        assert_eq!(
            action_after_vad_silence(true, true, None),
            SilenceAction::Flush
        );
    }
}
