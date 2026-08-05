//! Guards on the packaged-app Content-Security-Policy in `tauri.conf.json`.
//!
//! The CSP is only enforced on assets Tauri itself serves, so neither
//! `just dev` (loads the Vite `devUrl`) nor the Playwright suite (runs under
//! `vite preview`) can catch a policy that breaks the app. These tests pin the
//! non-obvious sources the frontend actually needs, so a future tightening
//! fails here instead of in a signed build.
//!
//! Kept as an integration test so the policy can be checked without the app
//! crate having to declare a test-only module.

use std::collections::HashMap;

const TAURI_CONF: &str = include_str!("../tauri.conf.json");

fn csp_directives() -> HashMap<String, Vec<String>> {
    let conf: serde_json::Value =
        serde_json::from_str(TAURI_CONF).expect("tauri.conf.json is valid JSON");
    let csp = conf["app"]["security"]["csp"]
        .as_str()
        .expect("app.security.csp is set as a policy string");

    csp.split(';')
        .filter_map(|directive| {
            let mut parts = directive.split_whitespace();
            let name = parts.next()?;
            Some((name.to_owned(), parts.map(str::to_owned).collect()))
        })
        .collect()
}

fn sources(directive: &str) -> Vec<String> {
    csp_directives()
        .remove(directive)
        .unwrap_or_else(|| panic!("csp is missing the {directive} directive"))
}

#[test]
fn script_src_allows_wasm_instantiation() {
    // Shiki's default engine (Oniguruma) instantiates inlined WebAssembly for
    // every code block; MediaPipe selfie segmentation does the same. Without
    // this token both silently degrade — highlighting drops to plain text and
    // animated avatars keep their background.
    assert!(sources("script-src").contains(&"'wasm-unsafe-eval'".to_owned()));
}

/// The `MEDIAPIPE_WASM_BASE` literal the frontend hands to `FilesetResolver`.
fn mediapipe_wasm_base() -> String {
    const CAPTURE: &str = include_str!("../../src/features/profile/lib/animatedAvatarCapture.ts");

    let after = CAPTURE
        .split_once("const MEDIAPIPE_WASM_BASE =")
        .expect("animatedAvatarCapture.ts declares MEDIAPIPE_WASM_BASE")
        .1;
    let url = after
        .split_once('"')
        .expect("MEDIAPIPE_WASM_BASE is a double-quoted string literal")
        .1;
    url.split_once('"')
        .expect("MEDIAPIPE_WASM_BASE literal is terminated")
        .0
        .to_owned()
}

#[test]
fn mediapipe_wasm_base_matches_the_installed_package() {
    // The CDN URL names a version, so the fetched wasm is only guaranteed to
    // match the bundled JS API if that version is the one npm installed. The
    // dependency is pinned exactly (no caret) so this comparison is meaningful
    // — a range would let the lockfile move underneath the URL.
    const PACKAGE_JSON: &str = include_str!("../../package.json");

    let manifest: serde_json::Value =
        serde_json::from_str(PACKAGE_JSON).expect("desktop/package.json is valid JSON");
    let installed = manifest["dependencies"]["@mediapipe/tasks-vision"]
        .as_str()
        .expect("desktop depends on @mediapipe/tasks-vision");
    assert!(
        !installed.starts_with(['^', '~', '>', '<', '*']),
        "@mediapipe/tasks-vision must be pinned exactly, found `{installed}`"
    );

    let base = mediapipe_wasm_base();
    let expected = format!("@mediapipe/tasks-vision@{installed}/wasm");
    assert!(
        base.ends_with(&expected),
        "MEDIAPIPE_WASM_BASE ({base}) must serve the installed version {installed}"
    );
}

#[test]
fn script_src_pins_the_mediapipe_loader_urls() {
    // `FilesetResolver.forVisionTasks` appends exactly one of these two files
    // (it probes for wasm SIMD at runtime) and loads it via a `<script>` tag,
    // so both must be allowed. Deriving them from the frontend constant keeps
    // a version bump in `animatedAvatarCapture.ts` from silently outrunning the
    // policy — the packaged app is the only place that would notice.
    let base = mediapipe_wasm_base();
    let allowed = sources("script-src");
    for loader in ["vision_wasm_internal.js", "vision_wasm_nosimd_internal.js"] {
        let url = format!("{base}/{loader}");
        assert!(
            allowed.contains(&url),
            "script-src must allow the pinned loader {url}"
        );
    }
}

/// Whether a `script-src` source names one specific script rather than a host
/// that could serve others. CSP keywords (`'self'`, `'wasm-unsafe-eval'`) pass;
/// so does a full URL ending in `.js`, as long as its host isn't a wildcard.
fn is_pinned_script_source(source: &str) -> bool {
    if source.starts_with('\'') {
        return true;
    }
    source.ends_with(".js") && !source.contains('*')
}

#[test]
fn script_src_trusts_no_bare_origins() {
    // The point of pinning full URLs: jsDelivr serves arbitrary npm and GitHub
    // content, so allowing the origin would let any renderer injection that can
    // append a `<script>` pull attacker-chosen code.
    for source in sources("script-src") {
        assert!(
            is_pinned_script_source(&source),
            "script-src must pin exact scripts, found broad source `{source}`"
        );
    }
}

#[test]
fn pinned_script_source_rejects_broad_sources() {
    // Guards the guard: the check above is only worth having if it fails on the
    // shapes that reopen the allowlist.
    for allowed in [
        "'self'",
        "'wasm-unsafe-eval'",
        "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@0.10.35/wasm/vision_wasm_internal.js",
    ] {
        assert!(is_pinned_script_source(allowed), "{allowed} should pass");
    }
    for rejected in [
        "https://cdn.jsdelivr.net",
        "https://cdn.jsdelivr.net/npm/",
        "https://*.jsdelivr.net/loader.js",
        "https:",
        "*",
    ] {
        assert!(
            !is_pinned_script_source(rejected),
            "{rejected} should be rejected"
        );
    }
}

#[test]
fn media_directives_allow_the_buzz_media_scheme() {
    // `rewriteRelayUrl` emits `buzz-media://localhost/...` until the loopback
    // proxy port resolves, so cold-start media renders through the custom
    // scheme (mapped to `http://buzz-media.localhost` on Windows).
    for directive in ["img-src", "media-src", "connect-src"] {
        let allowed = sources(directive);
        assert!(
            allowed.contains(&"buzz-media:".to_owned()),
            "{directive} must allow buzz-media:"
        );
        assert!(
            allowed.contains(&"http://buzz-media.localhost".to_owned()),
            "{directive} must allow http://buzz-media.localhost"
        );
    }
}

#[test]
fn connect_src_allows_ipc_and_cleartext_relays() {
    // `ipc:` / `http://ipc.localhost` carry every Tauri command. Cleartext
    // `http:`/`ws:` stay allowed because a relay URL is user-supplied and the
    // app accepts plain `ws://` on any host (`communityStorage::normalizeRelayUrl`,
    // the community edit form): `relayProbe` opens a browser WebSocket to it,
    // so narrowing this to loopback would report reachable relays as dead —
    // while the real connection, which runs through tauri-plugin-websocket in
    // Rust, is not governed by CSP at all. Blanket `https:` is already allowed,
    // so restricting the cleartext schemes would close no exfiltration path.
    let allowed = sources("connect-src");
    for source in [
        "ipc:",
        "http://ipc.localhost",
        "https:",
        "http:",
        "wss:",
        "ws:",
    ] {
        assert!(
            allowed.contains(&source.to_owned()),
            "connect-src must allow {source}"
        );
    }
}

#[test]
fn script_src_stays_free_of_unsafe_inline_and_eval() {
    let allowed = sources("script-src");
    // The inline boot script in index.html is covered by Tauri's build-time
    // sha256 hashing, so neither escape hatch is ever needed here.
    assert!(!allowed.contains(&"'unsafe-inline'".to_owned()));
    assert!(!allowed.contains(&"'unsafe-eval'".to_owned()));
}
