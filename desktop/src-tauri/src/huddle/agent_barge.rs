//! Periodic high-bar agent barge-in for huddles.
//!
//! When an agent's `agent_barge` toggle is on, about every 60s the host posts a
//! kind:9 prompt with a p-tag so the agent may chime in — but only under an
//! extremely high bar (clear open questions it can answer). Normal conversation
//! must stay silent.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Duration;

use uuid::Uuid;

use crate::{app_state::AppState, events};

use super::pipeline::{fetch_huddle_transcript_lines, sign_and_guard_stt_body};

/// How often barge-enabled agents are offered a chance to chime in.
pub(super) const AGENT_BARGE_INTERVAL: Duration = Duration::from_secs(60);

/// Strong silence-first instructions. Keep this terse: it is prepended to
/// recent transcript context every tick.
pub(super) const AGENT_BARGE_PROMPT: &str = "\
[Huddle auto-barge — SILENCE IS THE DEFAULT]
You were not directly addressed. Decide whether to speak at all.

Speak ONLY if ALL of the following are true:
1. There is a clear, unanswered open question in the recent huddle transcript.
2. You can answer it accurately in one or two short spoken sentences.
3. No human or agent has already answered it, and the conversation is not mid-turn.

Otherwise stay completely silent. Do not acknowledge this check. Do not summarize. \
Do not offer help. Do not say you are listening. Prefer silence over almost everything.

If you do speak, send one brief spoken reply to the current Context channel and stop.";

/// Spawn the barge ticker for one transcription/huddle generation.
///
/// Exits when `session_generation` advances (huddle ended or STT restarted).
pub(crate) fn spawn_agent_barge_task(
    channel_uuid: Uuid,
    session_generation: Arc<AtomicU64>,
    state: &AppState,
) {
    let spawned_gen = session_generation.load(Ordering::Acquire);
    let http_client = state.http_client.clone();
    let keys = match state.keys.lock() {
        Ok(k) => k.clone(),
        Err(_) => return,
    };
    let relay_base_url = crate::relay::relay_api_base_url_with_override(state);
    let app_handle = state
        .app_handle
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(AGENT_BARGE_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Wait a full interval before the first offer.
        interval.tick().await;

        let mut round_robin: usize = 0;
        loop {
            interval.tick().await;
            if session_generation.load(Ordering::Acquire) != spawned_gen {
                break;
            }

            let Some(handle) = app_handle.as_ref() else {
                continue;
            };
            use tauri::Manager;
            let Some(app_state) = handle.try_state::<AppState>() else {
                continue;
            };

            let barge_agents: Vec<String> = {
                let Ok(huddle) = app_state.huddle() else {
                    continue;
                };
                if !matches!(
                    huddle.phase,
                    super::HuddlePhase::Connected | super::HuddlePhase::Active
                ) {
                    continue;
                }
                let roster = huddle
                    .agent_pubkeys
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                roster
                    .into_iter()
                    .filter(|pk| {
                        huddle
                            .agent_voice_settings
                            .get(pk)
                            .is_some_and(|s| s.agent_barge)
                    })
                    .collect()
            };
            if barge_agents.is_empty() {
                continue;
            }

            // One agent per tick — avoids a chorus of barge replies.
            let target = &barge_agents[round_robin % barge_agents.len()];
            round_robin = round_robin.wrapping_add(1);

            let channel_lines =
                fetch_huddle_transcript_lines(&http_client, &keys, &relay_base_url, channel_uuid)
                    .await;
            let mut content = String::from(AGENT_BARGE_PROMPT);
            content.push_str("\n\n[Recent huddle transcript]\n");
            if channel_lines.is_empty() {
                content.push_str("(no recent transcript)\n");
            } else {
                // Keep the prompt bounded — last ~40 lines is enough context.
                let start = channel_lines.len().saturating_sub(40);
                for line in &channel_lines[start..] {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    content.push_str(trimmed);
                    content.push('\n');
                }
            }

            let p_tags = [target.as_str()];
            let builder = match events::build_message(
                channel_uuid,
                &content,
                None,
                &p_tags,
                &[],
                &[],
                &[],
                &[],
                None,
                &crate::relay::relay_api_base_url(),
            ) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("buzz-desktop: agent barge build_message: {e}");
                    continue;
                }
            };
            crate::relay_admission::wait_for_rate_limit().await;
            if session_generation.load(Ordering::Acquire) != spawned_gen {
                break;
            }
            let body_bytes = match sign_and_guard_stt_body(builder, &keys) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("buzz-desktop: agent barge publish: {e}");
                    continue;
                }
            };
            let url = format!("{relay_base_url}/events");
            let auth_header = match crate::relay::build_nip98_auth_header_for_keys(
                &keys,
                &reqwest::Method::POST,
                &url,
                &body_bytes,
            ) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("buzz-desktop: agent barge NIP-98 auth: {e}");
                    continue;
                }
            };
            match http_client
                .post(&url)
                .header("Authorization", auth_header)
                .header("Content-Type", "application/json")
                .body(body_bytes)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    eprintln!(
                        "buzz-desktop: agent barge offered to {}",
                        &target[..target.len().min(8)]
                    );
                }
                Ok(resp) => {
                    let msg = crate::relay::relay_error_message(resp).await;
                    eprintln!("buzz-desktop: agent barge post failed: {msg}");
                }
                Err(e) => {
                    eprintln!("buzz-desktop: agent barge post error: {e}");
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn barge_prompt_demands_silence_by_default() {
        let lower = AGENT_BARGE_PROMPT.to_ascii_lowercase();
        assert!(lower.contains("silence"));
        assert!(lower.contains("open question") || lower.contains("unanswered"));
        assert!(AGENT_BARGE_INTERVAL >= Duration::from_secs(60));
    }
}
