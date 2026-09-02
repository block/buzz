//! Rust-owned audio reconnect after an unexpected relay disconnect.
//!
//! Only the audio relay WebSocket is rebuilt. Huddle membership, mic capture,
//! STT/TTS, and agent voice stay live because `phase` never leaves `Active`;
//! the renderer sees progress through `HuddleState::audio_link` instead.
//!
//! The loop is fenced by huddle identity (`is_current_huddle`) after every
//! await: an intentional leave, or a replacement huddle started during the
//! backoff, always wins and any socket opened for the old huddle is cancelled
//! rather than installed.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;

use super::relay_api::{self, AudioRelayConnectError};
use super::state::{AudioLink, HuddleState};

/// Recovery, including pending dials, is bounded by this window. Long
/// enough to ride out a relay deploy (pod drain + Service endpoint
/// convergence) and a laptop Wi-Fi roam without dropping the huddle.
pub(crate) const RECONNECT_WINDOW: Duration = Duration::from_secs(60);
/// First backoff step after a failed dial; doubles per failure up to the ceiling.
const BACKOFF_BASE: Duration = Duration::from_millis(250);
const DIAL_TIMEOUT: Duration = Duration::from_secs(10);
const BACKOFF_MAX: Duration = Duration::from_secs(5);
/// A `huddle_relay_draining` refusal means a replacement pod is coming up, so
/// redial at the floor instead of growing the backoff.
const DRAINING_REDIAL_DELAY: Duration = BACKOFF_BASE;

/// Identity of the huddle whose audio socket is being rebuilt.
#[derive(Debug, Clone)]
pub(crate) struct ReconnectTarget {
    pub ephemeral_channel_id: String,
    pub parent_channel_id: Option<String>,
    huddle_generation: u64,
    lifetime: CancellationToken,
}

impl ReconnectTarget {
    pub(crate) fn capture(hs: &HuddleState, channel_id: &str, parent: Option<&str>) -> Self {
        Self {
            ephemeral_channel_id: channel_id.to_owned(),
            parent_channel_id: parent.map(str::to_owned),
            huddle_generation: hs.huddle_generation,
            lifetime: hs.huddle_cancel.clone(),
        }
    }
}

pub(crate) type AudioConnection = (CancellationToken, tokio::sync::mpsc::Sender<Vec<u8>>);

/// Entry point for the audio pipeline task when its socket exits unexpectedly.
/// Awaited in place by that task, so the loop ends with it; nothing is spawned.
///
/// Boxed because the call graph is recursive (pipeline task → reconnect →
/// `connect_audio_relay` → pipeline task) and the compiler needs a type-erased
/// edge to prove the future is `Send`.
pub(crate) fn after_unexpected_disconnect(
    app: tauri::AppHandle,
    target: ReconnectTarget,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    Box::pin(async move {
        use tauri::Manager;
        let state = app.state::<AppState>();
        let state: &AppState = &state;
        run(state, target, |target: ReconnectTarget| async move {
            relay_api::connect_audio_relay(
                &target.ephemeral_channel_id,
                target.parent_channel_id.as_deref(),
                state,
            )
            .await
        })
        .await;
    })
}

/// Claim recovery only for the pipeline that actually disconnected, never
/// whichever huddle happens to be current when its callback runs.
fn claim_reconnect(hs: &mut HuddleState, target: &ReconnectTarget) -> bool {
    if hs.audio_link != AudioLink::Live
        || target.lifetime.is_cancelled()
        || !hs.is_current_huddle(&target.ephemeral_channel_id, target.huddle_generation)
    {
        return false;
    }
    hs.audio_link = AudioLink::Reconnecting {
        attempt: 0,
        draining: false,
    };
    hs.audio_relay_pcm_tx = None;
    true
}

/// Drive redials with `dial` until one succeeds, the huddle ends or is
/// replaced, or the retry window is exhausted (`AudioLink::Lost`).
pub(crate) async fn run<F, Fut>(state: &AppState, target: ReconnectTarget, mut dial: F)
where
    F: FnMut(ReconnectTarget) -> Fut,
    Fut: Future<Output = Result<AudioConnection, AudioRelayConnectError>>,
{
    if !state
        .huddle()
        .map(|mut hs| claim_reconnect(&mut hs, &target))
        .unwrap_or(false)
    {
        return;
    }
    state.emit_huddle_state_changed();
    let deadline = Instant::now() + RECONNECT_WINDOW;
    let mut failures: u32 = 0;

    loop {
        // Do not start another dial at the budget boundary. The whole future
        // includes TCP/TLS/WS upgrade as well as the authenticated handshake.
        let result = if Instant::now() >= deadline {
            Err("audio recovery deadline elapsed".into())
        } else {
            tokio::select! {
                biased;
                result = tokio::time::timeout_at(
                    deadline.min(Instant::now() + DIAL_TIMEOUT), dial(target.clone())
                ) => result.unwrap_or_else(|_| Err("audio relay dial timed out".into())),
                _ = target.lifetime.cancelled() => return,
            }
        };

        // Lock scope is a block, not `drop()`: the guard must provably end
        // before the sleep for the future to stay `Send`.
        let next_delay = {
            let Ok(mut hs) = state.huddle() else { return };
            if target.lifetime.is_cancelled()
                || !hs.is_current_huddle(&target.ephemeral_channel_id, target.huddle_generation)
            {
                // Leave or replacement won while we were dialing; never resurrect.
                if let Ok((cancel, _)) = result {
                    cancel.cancel();
                }
                return;
            }
            // The replacement pipeline is spawned before its handles reach us
            // and cancels its own token on an unexpected exit. Its disconnect
            // callback cannot claim a reconnect while this loop owns the
            // link, so a token that is already cancelled here is a dial that
            // failed after auth — retry it, never install it as Live.
            let result = result.and_then(|conn| {
                if conn.0.is_cancelled() {
                    Err("replacement pipeline exited before it was installed".into())
                } else {
                    Ok(conn)
                }
            });
            match result {
                Ok((cancel, pcm_tx)) => {
                    if let Some(old_cancel) = hs.audio_ws_cancel.replace(cancel) {
                        old_cancel.cancel();
                    }
                    hs.audio_relay_pcm_tx = Some(pcm_tx);
                    hs.audio_link = AudioLink::Live;
                    None
                }
                Err(error) => {
                    failures += 1;
                    let draining = error.code() == Some("huddle_relay_draining");
                    eprintln!("buzz-desktop: huddle audio redial {failures} failed: {error}");
                    if Instant::now() >= deadline {
                        hs.audio_link = AudioLink::Lost;
                        None
                    } else {
                        hs.audio_link = AudioLink::Reconnecting {
                            attempt: failures,
                            draining,
                        };
                        Some(if draining {
                            DRAINING_REDIAL_DELAY
                        } else {
                            backoff_delay(failures, jitter_unit())
                        })
                    }
                }
            }
        };
        state.emit_huddle_state_changed();
        let Some(delay) = next_delay else { return };
        tokio::select! {
            biased;
            _ = target.lifetime.cancelled() => return,
            _ = tokio::time::sleep_until(deadline.min(Instant::now() + delay)) => {},
        }
        if !is_current(state, &target) {
            return;
        }
    }
}

fn is_current(state: &AppState, target: &ReconnectTarget) -> bool {
    state
        .huddle()
        .map(|hs| hs.is_current_huddle(&target.ephemeral_channel_id, target.huddle_generation))
        .unwrap_or(false)
}

/// Exponential delay for the `failures`-th consecutive failure, capped at
/// `BACKOFF_MAX`, then scaled into `[0.5, 1.0]` by `jitter` so clients that
/// lost the same pod do not redial in lockstep.
fn backoff_delay(failures: u32, jitter: f64) -> Duration {
    let exponent = failures.saturating_sub(1).min(16);
    let full = BACKOFF_BASE
        .saturating_mul(1_u32 << exponent)
        .min(BACKOFF_MAX);
    full.mul_f64(0.5 + 0.5 * jitter.clamp(0.0, 1.0))
}

fn jitter_unit() -> f64 {
    let mut bytes = [0u8; 4];
    // Entropy failure degrades to no jitter; the window is still honoured.
    let _ = getrandom::getrandom(&mut bytes);
    f64::from(u32::from_le_bytes(bytes)) / f64::from(u32::MAX)
}

#[cfg(test)]
#[path = "reconnect_tests.rs"]
mod tests;
