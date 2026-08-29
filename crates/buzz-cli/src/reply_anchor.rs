//! Turn-scoped reply anchor: the `--reply-to` a send defaults to.
//!
//! The harness (`buzz-acp`) tells agents in the prompt to pass `--reply-to`
//! so in-thread work stays in its thread. That is a soft guard — a model that
//! simply omits the flag posts to the channel root, and the thread's work leaks
//! into the channel. So the harness also writes the turn's anchor to
//! `$BUZZ_REPLY_ANCHOR_DIR/<channel-uuid>`, and this module applies it as the
//! default. Leaving the thread then requires saying so with `--channel-root`.
//!
//! Absent env var, absent file, or unreadable contents all mean "no anchor",
//! which is exactly the pre-existing behaviour: a human running `buzz` in a
//! terminal is unaffected.

use crate::error::CliError;

const ANCHOR_DIR_ENV: &str = "BUZZ_REPLY_ANCHOR_DIR";

/// Read the anchor the harness published for `channel_id`, if any.
///
/// A malformed file is ignored rather than fatal: the anchor is a convenience
/// the harness owns, and a corrupt one must not block the agent from replying.
fn published_anchor(channel_id: &str) -> Option<String> {
    let dir = std::env::var_os(ANCHOR_DIR_ENV)?;
    if dir.is_empty() {
        return None;
    }
    // `channel_id` reaches the filesystem, so accept only the UUID shape the
    // harness writes -- never a caller-supplied `../` traversal.
    if !is_uuid_shaped(channel_id) {
        return None;
    }
    let raw = std::fs::read_to_string(std::path::Path::new(&dir).join(channel_id)).ok()?;
    let anchor = raw.trim();
    is_hex64(anchor).then(|| anchor.to_owned())
}

fn is_uuid_shaped(s: &str) -> bool {
    s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Resolve the effective `--reply-to` for a send.
///
/// Precedence: an explicit `--channel-root` wins (posts at the root), then an
/// explicit `--reply-to`, then the harness anchor. Passing both explicit flags
/// is a contradiction and is rejected rather than silently resolved -- the two
/// spellings mean opposite things, and guessing which one the agent meant is
/// how a message ends up in the wrong place.
pub fn resolve_reply_to(
    channel_id: &str,
    reply_to: Option<String>,
    channel_root: bool,
) -> Result<Option<String>, CliError> {
    match (reply_to, channel_root) {
        (Some(_), true) => Err(CliError::Usage(
            "--reply-to and --channel-root are mutually exclusive: --reply-to threads the \
             message, --channel-root posts it at the channel root"
                .into(),
        )),
        (explicit @ Some(_), false) => Ok(explicit),
        (None, true) => Ok(None),
        (None, false) => Ok(published_anchor(channel_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CH: &str = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
    const ANCHOR: &str = "aa11bb22cc33dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66";
    const EXPLICIT: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    /// The env var is process-wide, so anchor-reading tests must not run
    /// concurrently with each other.
    fn with_anchor_dir<T>(body: impl FnOnce(&std::path::Path) -> T) -> T {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var(ANCHOR_DIR_ENV, dir.path());
        let out = body(dir.path());
        std::env::remove_var(ANCHOR_DIR_ENV);
        out
    }

    /// The whole point: omitting the flag must still thread the reply.
    #[test]
    fn omitted_reply_to_inherits_the_published_anchor() {
        with_anchor_dir(|dir| {
            std::fs::write(dir.join(CH), ANCHOR).expect("write anchor");
            assert_eq!(
                resolve_reply_to(CH, None, false).expect("resolve"),
                Some(ANCHOR.to_string())
            );
        });
    }

    /// Leaving the thread has to be said out loud.
    #[test]
    fn channel_root_opts_out_of_the_anchor() {
        with_anchor_dir(|dir| {
            std::fs::write(dir.join(CH), ANCHOR).expect("write anchor");
            assert_eq!(resolve_reply_to(CH, None, true).expect("resolve"), None);
        });
    }

    /// An explicit target still wins -- replying deeper in a thread stays possible.
    #[test]
    fn explicit_reply_to_overrides_the_anchor() {
        with_anchor_dir(|dir| {
            std::fs::write(dir.join(CH), ANCHOR).expect("write anchor");
            assert_eq!(
                resolve_reply_to(CH, Some(EXPLICIT.into()), false).expect("resolve"),
                Some(EXPLICIT.to_string())
            );
        });
    }

    #[test]
    fn explicit_reply_to_with_channel_root_is_rejected() {
        assert!(resolve_reply_to(CH, Some(EXPLICIT.into()), true).is_err());
    }

    /// A human at a terminal has no anchor dir and must be unaffected.
    #[test]
    fn no_env_means_no_anchor() {
        std::env::remove_var(ANCHOR_DIR_ENV);
        assert_eq!(resolve_reply_to(CH, None, false).expect("resolve"), None);
    }

    /// A channel with no in-flight anchor posts at the root, as before.
    #[test]
    fn missing_anchor_file_means_no_anchor() {
        with_anchor_dir(|_| {
            assert_eq!(resolve_reply_to(CH, None, false).expect("resolve"), None);
        });
    }

    /// Garbage must not become a bogus `--reply-to` that the relay rejects.
    #[test]
    fn malformed_anchor_is_ignored() {
        with_anchor_dir(|dir| {
            std::fs::write(dir.join(CH), "not-an-event-id").expect("write anchor");
            assert_eq!(resolve_reply_to(CH, None, false).expect("resolve"), None);
        });
    }

    /// The channel id is used as a path segment; traversal must not escape.
    #[test]
    fn non_uuid_channel_never_touches_the_filesystem() {
        with_anchor_dir(|_| {
            assert_eq!(
                resolve_reply_to("../../etc/passwd", None, false).expect("resolve"),
                None
            );
        });
    }
}
