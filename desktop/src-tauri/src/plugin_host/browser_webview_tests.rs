//! Source guard proving the browser adapter never gains a route into Buzz
//! through Tauri IPC, an injected script, a custom protocol, or evaluated
//! page script (see docs/plugin-browser-prototype.md). Declared outside the
//! `#[cfg(target_os = "macos")]` module gate in `browser_webview.rs` so this
//! guard runs on Linux CI even though the adapter implementation it checks
//! only compiles on macOS.

const SOURCE: &str = include_str!("browser_webview.rs");

const BANNED_TOKENS: &[&str] = &[
    "with_ipc_handler",
    "with_initialization_script",
    "with_custom_protocol",
    "WebviewBuilder",
    "add_child",
    "evaluate_script",
];

#[test]
fn adapter_never_uses_tauri_ipc_injected_script_or_evaluate_script() {
    for token in BANNED_TOKENS {
        assert!(
            !SOURCE.contains(token),
            "browser_webview.rs must never contain `{token}`; \
             the adapter injects no script and never wraps Tauri's own child-webview builder"
        );
    }
}

#[test]
fn source_guard_is_falsifiable() {
    // A guard that can never fail protects nothing. Prove the exact
    // detection mechanism this test relies on actually trips.
    let sample = "fn demo() { widget.evaluate_script(\"1\"); }";
    assert!(
        BANNED_TOKENS.iter().any(|token| sample.contains(token)),
        "the banned-token scan must be able to detect a real occurrence"
    );
}
