//! [`crate::driver::Driver`] implementation for muse.ai.
//!
//! One Buzz channel session maps to one browser tab, so each channel keeps
//! its own continuous Muse conversation — the same continuity a user gets
//! chatting in the browser. The framed prompt buzz-acp sends (channel
//! context, instructions, history pointers) is forwarded verbatim: the
//! bridge adds no paraphrase between the channel and Muse. On the first
//! turn of a session the `systemPrompt` from `session/new` is prepended;
//! later turns send only the new message, since the tab already holds the
//! conversation.
//!
//! Stream tracking: the driver polls the probe in [`crate::selectors`]
//! and emits only text growth. It distinguishes a *new* assistant message
//! (message count increased) from growth of the current one, and it
//! recognizes the user's own echoed message (text equal to what was
//! submitted) so the echo is never relayed back to the channel as Muse's
//! reply. A wholesale rewrite of the tracked text (e.g. a markdown
//! re-render) resets the baseline silently rather than duplicating already
//! emitted text or panicking on a byte boundary.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, Mutex};
use tracing::{debug, info, warn};

use crate::auth;
use crate::chrome::{self, Browser, Tab};
use crate::driver::{Driver, TurnOutcome};
use crate::selectors;
use crate::BridgeError;

/// Poll interval for the response probe during a turn.
const POLL_INTERVAL: Duration = Duration::from_millis(800);
/// How long a headed first-run sign-in wait lasts before giving up.
const LOGIN_WAIT: Duration = Duration::from_secs(600);
/// How long a page may take to present a composer before the turn fails.
const LOAD_WAIT: Duration = Duration::from_secs(60);
/// How long a turn may show zero Muse activity (no stop control, no text
/// growth) before it fails loudly instead of hanging.
const NO_PROGRESS_LIMIT: Duration = Duration::from_secs(120);
/// Consecutive stable polls (no stop control, no text growth) that settle
/// a turn, once activity was observed.
const STABLE_POLLS_TO_FINISH: u32 = 3;

/// Environment override for headed/headless: `0`/`false` forces a visible
/// window (needed for first-run sign-in), `1`/`true` forces headless.
const HEADLESS_ENV: &str = "MUSE_ACP_HEADLESS";

fn want_headless() -> bool {
    match std::env::var(HEADLESS_ENV).ok().as_deref().map(str::trim) {
        Some("0") | Some("false") | Some("no") => false,
        Some("1") | Some("true") | Some("yes") => true,
        _ => auth::has_saved_session(),
    }
}

/// The Muse web driver: one Chromium instance, one tab per session.
pub struct MuseWebDriver {
    /// Serializes session creation (browser launch + tab + login wait are
    /// global to the one browser profile).
    creation: Mutex<()>,
    browser: Mutex<Option<Browser>>,
    tabs: Mutex<HashMap<String, Tab>>,
}

impl MuseWebDriver {
    /// Create a driver. No browser is launched until the first session.
    pub fn new() -> Self {
        Self {
            creation: Mutex::new(()),
            browser: Mutex::new(None),
            tabs: Mutex::new(HashMap::new()),
        }
    }

    async fn tab_for(&self, session_id: &str) -> Option<Tab> {
        self.tabs.lock().await.get(session_id).cloned()
    }

    async fn forget_tab(&self, session_id: &str) {
        if let Some(tab) = self.tabs.lock().await.remove(session_id) {
            let _ = tab.close().await;
        }
    }

    /// Wait for a signed-in composer in a fresh tab. Headed first runs
    /// wait for the user to sign in; headless runs without a saved session
    /// fail immediately with instructions instead of hanging.
    async fn wait_for_ready(&self, tab: &Tab, headless: bool) -> Result<(), BridgeError> {
        let start = Instant::now();
        let mut sign_in_announced = false;
        loop {
            let probe = tab.evaluate_json(selectors::PROBE_JS).await?;
            let composer = probe.get("composer").and_then(|c| c.as_str());
            let login_wall = probe
                .get("loginWall")
                .and_then(|l| l.as_bool())
                .unwrap_or(false);
            match (composer, login_wall) {
                (Some(selector), _) => {
                    debug!(composer = selector, "muse composer ready");
                    if let Err(e) = auth::mark_signed_in() {
                        warn!("could not record Muse session marker: {e}");
                    }
                    return Ok(());
                }
                (None, true) => {
                    if headless {
                        return Err(BridgeError::NotSignedIn(
                            "no saved Muse session; run once with MUSE_ACP_HEADLESS=0 and sign in to Muse in the opened browser window".into(),
                        ));
                    }
                    if !sign_in_announced {
                        info!(
                            "No saved Muse session. Sign in to Muse in the opened browser window; \
                             this turn continues once the composer appears."
                        );
                        sign_in_announced = true;
                    }
                    if start.elapsed() > LOGIN_WAIT {
                        return Err(BridgeError::NotSignedIn(
                            "sign-in wait expired without a Muse composer appearing".into(),
                        ));
                    }
                }
                (None, false) => {
                    if start.elapsed() > LOAD_WAIT {
                        return Err(BridgeError::WebDom(
                            "Muse composer never appeared and no login wall was detected; \
                             the page may have changed or failed to load"
                                .into(),
                        ));
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    async fn ensure_session_inner(&self, session_id: &str) -> Result<Tab, BridgeError> {
        if let Some(tab) = self.tab_for(session_id).await {
            return Ok(tab);
        }
        let _creation = self.creation.lock().await;
        // A concurrent creation may have finished while we waited.
        if let Some(tab) = self.tab_for(session_id).await {
            return Ok(tab);
        }
        let mut browser_guard = self.browser.lock().await;
        if browser_guard.is_none() {
            let profile = auth::profile_dir().ok_or_else(|| {
                BridgeError::NotSignedIn(
                    "cannot determine a profile directory for the Muse browser session".into(),
                )
            })?;
            let headless = want_headless();
            info!(headless, profile = %profile.display(), "launching Chromium for Muse");
            let browser = chrome::launch(headless, &profile).await?;
            *browser_guard = Some(browser);
        }
        let headless = want_headless();
        let browser = browser_guard
            .as_ref()
            .ok_or_else(|| BridgeError::ChromeLaunch("browser vanished during launch".into()))?;
        let tab = browser.new_tab(selectors::MUSE_CHAT_URL).await?;
        drop(browser_guard);
        self.tabs
            .lock()
            .await
            .insert(session_id.to_string(), tab.clone());
        if let Err(e) = self.wait_for_ready(&tab, headless).await {
            self.forget_tab(session_id).await;
            return Err(e);
        }
        // Best-effort fresh conversation; absence is non-fatal (see
        // selectors::NEW_CHAT_JS): the tab's conversation is still coherent.
        match tab.evaluate_json(selectors::NEW_CHAT_JS).await {
            Ok(value) => {
                debug!(new_chat = %value, "new-chat step result");
            }
            Err(e) => {
                debug!("new-chat step failed (non-fatal): {e}");
            }
        }
        Ok(tab)
    }

    async fn run_turn_inner(
        &self,
        session_id: &str,
        submitted_text: &str,
        cancel: Arc<AtomicBool>,
        chunks: mpsc::Sender<String>,
    ) -> Result<TurnOutcome, BridgeError> {
        let tab = self.ensure_session_inner(session_id).await?;
        if cancel.load(Ordering::SeqCst) {
            return Ok(TurnOutcome::Cancelled);
        }
        // Baseline: the last message visible before this turn, so the
        // previous reply is never re-emitted as the new one.
        let baseline = tab.evaluate_json(selectors::PROBE_JS).await?;
        let mut prev_text = baseline
            .get("responseText")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let mut prev_count = baseline.get("responseCount").and_then(|c| c.as_u64());

        let submitted = tab
            .evaluate_json(&selectors::submit_js(submitted_text))
            .await?;
        let submitted_ok = submitted
            .get("submitted")
            .and_then(|s| s.as_bool())
            .unwrap_or(false);
        let via = submitted
            .get("via")
            .and_then(|v| v.as_str())
            .unwrap_or("none");
        let composer = submitted
            .get("composer")
            .and_then(|c| c.as_str())
            .unwrap_or("?");
        debug!(
            submitted = submitted_ok,
            via, composer, "prompt submitted to Muse"
        );
        if !submitted_ok {
            return Err(BridgeError::WebDom(format!(
                "could not submit prompt: composer matched {composer} but no send path fired"
            )));
        }

        let mut emitted_bytes: usize = 0;
        let mut stable_polls: u32 = 0;
        let mut saw_activity = false;
        let mut last_progress = Instant::now();
        let mut fallback_warned = false;
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = tab.evaluate_json(selectors::STOP_JS).await;
                return Ok(TurnOutcome::Cancelled);
            }
            let probe = tab.evaluate_json(selectors::PROBE_JS).await?;
            if probe
                .get("loginWall")
                .and_then(|l| l.as_bool())
                .unwrap_or(false)
            {
                auth::clear_saved_session();
                return Err(BridgeError::NotSignedIn(
                    "Muse login expired mid-turn; the next turn waits for sign-in again".into(),
                ));
            }
            let mode = probe
                .get("responseMode")
                .and_then(|m| m.as_str())
                .unwrap_or("none");
            if mode == "fallback-main" && !fallback_warned {
                warn!(
                    "tracking Muse's reply via the main-element fallback; \
                     verify the message selectors in selectors.rs"
                );
                fallback_warned = true;
            }
            let text = probe
                .get("responseText")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            let count = probe.get("responseCount").and_then(|c| c.as_u64());
            let stop_present = probe
                .get("stopPresent")
                .and_then(|s| s.as_bool())
                .unwrap_or(false);
            if stop_present {
                saw_activity = true;
            }

            // A new message element means a new reply started: reset the
            // baseline so the fresh reply streams from its first byte.
            if count != prev_count {
                // ...unless the "new" message is just the user's own echoed
                // prompt, which must never be relayed as Muse's reply.
                if text.trim() == submitted_text.trim() {
                    // Clone: `text` is still needed below for growth math.
                    prev_text = text.clone();
                    prev_count = count;
                    last_progress = Instant::now();
                } else {
                    prev_text = String::new();
                    prev_count = count;
                }
            }

            // Growth is measured before the baseline moves: comparing
            // lengths after updating prev_text would count every poll as
            // stable and end turns while Muse is still streaming.
            let grew = text.len() > prev_text.len() && text.starts_with(&prev_text);
            let rewritten = !grew && text != prev_text;
            if grew {
                let delta = text[prev_text.len()..].to_string();
                prev_text = text;
                last_progress = Instant::now();
                saw_activity = true;
                if !delta.is_empty() {
                    emitted_bytes += delta.len();
                    if chunks.send(delta).await.is_err() {
                        return Ok(TurnOutcome::Cancelled);
                    }
                }
            } else if rewritten {
                // Wholesale rewrite (e.g. a markdown re-render): reset the
                // baseline silently. Re-emitting would duplicate text; the
                // growth after the rewrite still streams normally.
                prev_text = text;
                last_progress = Instant::now();
            }

            if !stop_present {
                if grew || rewritten {
                    stable_polls = 0;
                } else {
                    stable_polls += 1;
                }
                if stable_polls >= STABLE_POLLS_TO_FINISH && (saw_activity || emitted_bytes > 0) {
                    return Ok(TurnOutcome::Finished);
                }
            } else {
                stable_polls = 0;
            }

            if last_progress.elapsed() > NO_PROGRESS_LIMIT && !saw_activity && emitted_bytes == 0 {
                return Err(BridgeError::WebDom(
                    "no Muse response observed after submit; the composer/send selectors may be stale"
                        .into(),
                ));
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}

impl Default for MuseWebDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver for MuseWebDriver {
    fn ensure_session(
        &self,
        session_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), BridgeError>> + Send + '_>> {
        let session_id = session_id.to_string();
        Box::pin(async move {
            self.ensure_session_inner(&session_id).await?;
            Ok(())
        })
    }

    fn run_turn(
        &self,
        session_id: &str,
        message: &str,
        cancel: Arc<AtomicBool>,
        chunks: mpsc::Sender<String>,
    ) -> Pin<Box<dyn Future<Output = Result<TurnOutcome, BridgeError>> + Send + '_>> {
        let session_id = session_id.to_string();
        let message = message.to_string();
        Box::pin(async move {
            self.run_turn_inner(&session_id, &message, cancel, chunks)
                .await
        })
    }
}
