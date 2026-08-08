use super::HuddlePhase;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum AgentTtsRuntimeGate {
    Disabled,
    Inactive,
    NeedsPipeline,
    Ready,
}

pub(super) fn classify_agent_tts_runtime(
    enabled: bool,
    phase: &HuddlePhase,
    has_pipeline: bool,
) -> AgentTtsRuntimeGate {
    if !enabled {
        AgentTtsRuntimeGate::Disabled
    } else if !matches!(phase, HuddlePhase::Connected | HuddlePhase::Active) {
        AgentTtsRuntimeGate::Inactive
    } else if has_pipeline {
        AgentTtsRuntimeGate::Ready
    } else {
        AgentTtsRuntimeGate::NeedsPipeline
    }
}

/// Maximum text length accepted for TTS synthesis.
/// ~2000 chars is 1–2 minutes of speech. Longer messages are truncated.
pub(super) const MAX_TTS_TEXT_LEN: usize = 2000;

pub(super) fn normalize_agent_tts_text(text: String) -> String {
    if text.chars().count() > MAX_TTS_TEXT_LEN {
        let mut truncated: String = text.chars().take(MAX_TTS_TEXT_LEN).collect();
        truncated.push_str("... message truncated.");
        truncated
    } else {
        text
    }
}

pub(super) async fn enqueue_agent_tts_text<F>(
    route_id: u64,
    text: String,
    enqueue: F,
) -> Result<(), String>
where
    F: FnOnce(u64, String) -> Result<(), String> + Send + 'static,
{
    tokio::task::spawn_blocking(move || enqueue(route_id, text))
        .await
        .map_err(|error| format!("TTS enqueue task failed: {error}"))?
}

#[cfg(test)]
#[path = "agent_tts_routing_tests.rs"]
mod tests;
