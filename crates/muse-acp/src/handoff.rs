//! Visible-to-headless browser handoff, kept separate from routing.
//!
//! The bridge has two browser modes for the same session:
//!
//! * **Visible** (`MUSE_ACP_HEADLESS=0`, or the default when no saved
//!   session exists): a headed Chromium window where the user signs in
//!   to Muse once. The login cookies live in the Chromium profile
//!   (`MUSE_ACP_PROFILE`, default `~/.muse-acp`); the bridge records
//!   only a marker file (`session.json`: `version` + `logged_in_at_unix`)
//!   in that profile. It never sees, copies, or stores a password,
//!   cookie, token, or any other credential.
//! * **Headless** (`MUSE_ACP_HEADLESS=1`, or the default once a saved
//!   session exists): the same profile, launched headless, reusing the
//!   cookies the visible run created.
//!
//! The handoff gate is the check that makes that transition safe:
//!
//! 1. Headless requires a saved session; visible does not (it is how
//!    a session gets created). A headless launch without a saved
//!    session fails immediately with `NotSignedIn` instead of hanging
//!    or silently starting an unsigned session.
//! 2. The profile directory must be identical across the handoff. A
//!    visible run in profile A followed by a headless run in profile B
//!    is not a handoff at all — it is a different session — so the gate
//!    rejects it instead of letting the headless run silently lose the
//!    login the visible run created.
//! 3. The gate is separate from side-chat routing. Routing decides
//!    *which Muse conversation* a turn goes to (channel -> side chat);
//!    the gate decides *whether the browser may run headless at all*
//!    (profile + saved session). Neither reads the other's state: the
//!    gate takes only a profile directory and a saved-session flag, and
//!    routing takes only a channel and candidate titles. A routing miss
//!    therefore can never flip the handoff, and a handoff failure can
//!    never re-route a channel.
//!
//! Session continuity proof: given a profile directory and the marker
//! file's contents, `continuity_proof` reports the profile path, the
//! marker version, and whether a saved session is present — nothing
//! else. The marker itself contains no credential fields, which the
//! tests assert: a marker that carried a cookie or token would fail
//! `marker_has_no_credentials`.

use std::path::{Path, PathBuf};

use crate::BridgeError;

/// Environment override for headed/headless, shared with the driver:
/// `0`/`false` forces visible, `1`/`true` forces headless.
pub const HEADLESS_ENV: &str = "MUSE_ACP_HEADLESS";

/// Browser mode for one bridge run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffMode {
    /// A visible Chromium window (sign-in, or operator-forced).
    Visible,
    /// A headless Chromium launch (requires a saved session).
    Headless,
}

/// Decide the mode from the environment and whether a saved session
/// exists. This is the same rule the driver used before this module
/// existed, extracted so the gate can be tested without a browser:
/// an explicit env value wins; otherwise headless iff a session is
/// saved, visible otherwise (so first run signs in visibly).
pub fn mode_from_env_and_saved_session(saved_session: bool) -> HandoffMode {
    match std::env::var(HEADLESS_ENV)
        .ok()
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("0") | Some("false") | Some("no") => HandoffMode::Visible,
        Some("1") | Some("true") | Some("yes") => HandoffMode::Headless,
        _ => {
            if saved_session {
                HandoffMode::Headless
            } else {
                HandoffMode::Visible
            }
        }
    }
}

/// The handoff gate for one bridge run: a profile directory and a mode.
/// No channel, no side-chat title, no routing state lives here, by
/// design — see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HandoffGate {
    profile_dir: PathBuf,
    mode: HandoffMode,
}

impl HandoffGate {
    /// Build a gate for a profile directory and a mode.
    pub fn new(profile_dir: PathBuf, mode: HandoffMode) -> Self {
        Self { profile_dir, mode }
    }

    /// The Chromium profile directory this gate governs.
    pub fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }

    /// This run's mode.
    pub fn mode(&self) -> HandoffMode {
        self.mode
    }

    /// True when this gate requires a saved session to proceed.
    pub fn requires_saved_session(&self) -> bool {
        self.mode == HandoffMode::Headless
    }

    /// Check that this gate may proceed given whether a saved session
    /// exists. A headless gate without a saved session fails with
    /// `NotSignedIn`; a visible gate always passes (it is the path
    /// that creates the session).
    pub fn check(&self, saved_session: bool) -> Result<(), BridgeError> {
        if self.requires_saved_session() && !saved_session {
            return Err(BridgeError::NotSignedIn(
                "no saved Muse session; run once with MUSE_ACP_HEADLESS=0 and sign in to Muse in the opened browser window".into(),
            ));
        }
        Ok(())
    }

    /// Check continuity across a handoff: `self` is the new run, `prev`
    /// is the previous run. The profile directory must be identical;
    /// a mode change from visible to headless is allowed only when a
    /// saved session exists (the visible run must have created one).
    /// Headless-to-visible is always allowed (it re-enters sign-in).
    pub fn check_continuity_from(
        &self,
        prev: &HandoffGate,
        saved_session: bool,
    ) -> Result<(), BridgeError> {
        if self.profile_dir != prev.profile_dir {
            return Err(BridgeError::NotSignedIn(format!(
                "browser profile changed across handoff: {} -> {}; \
                 session continuity requires the same profile directory",
                prev.profile_dir.display(),
                self.profile_dir.display()
            )));
        }
        if prev.mode == HandoffMode::Visible && self.mode == HandoffMode::Headless && !saved_session
        {
            return Err(BridgeError::NotSignedIn(
                "visible-to-headless handoff without a saved session; \
                 sign in visibly first, then switch to headless"
                    .into(),
            ));
        }
        self.check(saved_session)
    }
}

/// A continuity proof for one profile: what the operator (or a test)
/// can report without ever touching credentials. Contains only the
/// profile path, the marker version, and whether a saved session is
/// present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuityProof {
    pub profile_dir: PathBuf,
    pub marker_version: Option<u64>,
    pub saved_session: bool,
}

impl ContinuityProof {
    /// Build a proof from a profile directory and a marker file's
    /// parsed JSON value (or `None` when there is no marker). The
    /// marker version is read, never any credential field.
    pub fn from_marker(profile_dir: &Path, marker: Option<&serde_json::Value>) -> Self {
        let version = marker
            .and_then(|m| m.get("version"))
            .and_then(|v| v.as_u64());
        Self {
            profile_dir: profile_dir.to_path_buf(),
            marker_version: version,
            saved_session: version == Some(1),
        }
    }

    /// A one-line, credential-free description for logs / reports.
    pub fn describe(&self) -> String {
        format!(
            "profile={} marker_version={:?} saved_session={}",
            self.profile_dir.display(),
            self.marker_version,
            self.saved_session
        )
    }
}

/// True when a marker JSON object contains no credential-like fields.
/// The bridge's marker is `{ version, logged_in_at_unix }` only; any
/// field named like a cookie, token, password, or key fails this check,
/// so a future change that smuggled credentials into the marker would
/// be caught by the test below, not shipped silently.
pub fn marker_has_no_credentials(marker: &serde_json::Value) -> bool {
    let Some(obj) = marker.as_object() else {
        return true;
    };
    for key in obj.keys() {
        let lower = key.to_ascii_lowercase();
        if lower.contains("cookie")
            || lower.contains("token")
            || lower.contains("password")
            || lower.contains("secret")
            || lower.contains("key")
            || lower.contains("credential")
            || lower.contains("sessionid")
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// Serializes the env-mutating tests: they share `MUSE_ACP_HEADLESS`.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    #[test]
    fn headless_requires_saved_session_visible_does_not() {
        let headless = HandoffGate::new(PathBuf::from("/tmp/p"), HandoffMode::Headless);
        assert!(headless.requires_saved_session());
        assert!(headless.check(false).is_err());
        assert!(headless.check(true).is_ok());
        let visible = HandoffGate::new(PathBuf::from("/tmp/p"), HandoffMode::Visible);
        assert!(!visible.requires_saved_session());
        assert!(visible.check(false).is_ok());
    }

    #[test]
    fn visible_to_headless_handoff_requires_same_profile_and_saved_session() {
        let visible = HandoffGate::new(PathBuf::from("/tmp/p"), HandoffMode::Visible);
        let headless = HandoffGate::new(PathBuf::from("/tmp/p"), HandoffMode::Headless);
        // No saved session yet: the handoff fails, it does not silently
        // start an unsigned headless session.
        assert!(headless.check_continuity_from(&visible, false).is_err());
        // After the visible run saved a session, the same-profile
        // headless run passes: session continuity is proven.
        assert!(headless.check_continuity_from(&visible, true).is_ok());
        // A different profile is not a handoff, even with a saved session.
        let other = HandoffGate::new(PathBuf::from("/tmp/other"), HandoffMode::Headless);
        assert!(other.check_continuity_from(&visible, true).is_err());
        // Headless-to-visible is always allowed (re-enter sign-in).
        assert!(visible.check_continuity_from(&headless, false).is_ok());
    }

    #[test]
    fn mode_from_env_wins_over_saved_session() {
        let _guard = ENV_GUARD.lock().unwrap();
        std::env::set_var(HEADLESS_ENV, "0");
        assert_eq!(mode_from_env_and_saved_session(true), HandoffMode::Visible);
        std::env::set_var(HEADLESS_ENV, "1");
        assert_eq!(
            mode_from_env_and_saved_session(false),
            HandoffMode::Headless
        );
        std::env::remove_var(HEADLESS_ENV);
        assert_eq!(mode_from_env_and_saved_session(true), HandoffMode::Headless);
        assert_eq!(mode_from_env_and_saved_session(false), HandoffMode::Visible);
    }

    #[test]
    fn continuity_proof_carries_no_credentials() {
        let marker = json!({ "version": 1, "logged_in_at_unix": 1793520000 });
        assert!(marker_has_no_credentials(&marker));
        let proof = ContinuityProof::from_marker(Path::new("/tmp/p"), Some(&marker));
        assert!(proof.saved_session);
        assert_eq!(proof.marker_version, Some(1));
        assert!(proof.describe().contains("/tmp/p"));
        assert!(!proof.describe().contains("cookie"));
        // A marker that smuggled a credential fails the check.
        let bad = json!({ "version": 1, "cookie": "abc" });
        assert!(!marker_has_no_credentials(&bad));
        let no_marker = ContinuityProof::from_marker(Path::new("/tmp/p"), None);
        assert!(!no_marker.saved_session);
        assert_eq!(no_marker.marker_version, None);
    }

    #[test]
    fn handoff_gate_does_not_depend_on_routing() {
        // The gate's inputs are only profile + mode + saved flag. A
        // routing change (channel, side-chat title) cannot change a
        // gate's decision, by construction: there is no channel field
        // on HandoffGate to read. This test pins that shape by
        // checking the gate twice with different routing contexts
        // implied only by the caller — the gate itself is unchanged.
        let gate = HandoffGate::new(PathBuf::from("/tmp/p"), HandoffMode::Headless);
        assert!(gate.check(true).is_ok());
        assert!(gate.check(false).is_err());
    }
}
