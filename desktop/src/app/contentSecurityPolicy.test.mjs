// Guards on the packaged-app Content-Security-Policy in `tauri.conf.json`.
//
// The CSP is only enforced on assets Tauri itself serves, so neither
// `just dev` (loads the Vite `devUrl`) nor the Playwright suite (runs under
// `vite preview`) can catch a policy that breaks the app. These tests pin the
// non-obvious sources the frontend actually needs, so a future tightening
// fails here instead of in a signed build.
//
// Lives on the frontend side because it checks the policy against frontend
// code (`MEDIAPIPE_WASM_BASE`), and the `desktop/**` CI filter covers both
// that code and `src-tauri/tauri.conf.json`.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { MEDIAPIPE_WASM_BASE } from "../features/profile/lib/animatedAvatarCapture.ts";

const TAURI_CONF = JSON.parse(
  readFileSync(
    new URL("../../src-tauri/tauri.conf.json", import.meta.url),
    "utf8",
  ),
);

function cspDirectives() {
  const csp = TAURI_CONF.app?.security?.csp;
  assert.equal(
    typeof csp,
    "string",
    "app.security.csp is set as a policy string",
  );
  const directives = new Map();
  for (const directive of csp.split(";")) {
    const [name, ...sources] = directive.split(/\s+/).filter(Boolean);
    if (name) directives.set(name, sources);
  }
  return directives;
}

function sources(directive) {
  const allowed = cspDirectives().get(directive);
  assert.ok(allowed, `csp is missing the ${directive} directive`);
  return allowed;
}

/** The npm scope the MediaPipe loader must come from. A CSP source ending in
 * `/` is a path *prefix* — paths can't be wildcarded — so this admits any
 * `@mediapipe` package while excluding the rest of what jsDelivr serves. */
const MEDIAPIPE_SCOPE = "https://cdn.jsdelivr.net/npm/@mediapipe/";

/** How many path segments a source narrows to past its origin. */
function pathDepth(source) {
  const authority = source.split("://")[1];
  if (authority === undefined) return 0;
  const slash = authority.indexOf("/");
  if (slash === -1) return 0;
  return authority
    .slice(slash + 1)
    .split("/")
    .filter(Boolean).length;
}

/** Whether a `script-src` source is narrow enough to be worth allowing. CSP
 * keywords (`'self'`, `'wasm-unsafe-eval'`) pass. A remote source must name a
 * non-wildcard host *and* a path reaching at least a publisher scope — a bare
 * origin, a scheme, or a registry root would put arbitrary third-party code
 * one injected `<script>` away. */
function isPinnedScriptSource(source) {
  if (source.startsWith("'")) return true;
  return !source.includes("*") && pathDepth(source) >= 2;
}

test("script-src allows wasm instantiation", () => {
  // Shiki's default engine (Oniguruma) instantiates inlined WebAssembly for
  // every code block; MediaPipe selfie segmentation does the same. Without
  // this token both silently degrade — highlighting drops to plain text and
  // animated avatars keep their background.
  assert.ok(sources("script-src").includes("'wasm-unsafe-eval'"));
});

test("script-src scopes the MediaPipe loader", () => {
  // `FilesetResolver.forVisionTasks` loads `vision_wasm[_nosimd]_internal.js`
  // via a `<script>` tag (which of the two depends on a runtime SIMD probe),
  // so the loader URL the frontend builds has to fall inside the allowlisted
  // prefix — checked here rather than discovered in a signed build.
  assert.ok(
    sources("script-src").includes(MEDIAPIPE_SCOPE),
    `script-src must allow ${MEDIAPIPE_SCOPE}`,
  );
  assert.ok(
    MEDIAPIPE_WASM_BASE.startsWith(MEDIAPIPE_SCOPE),
    `MEDIAPIPE_WASM_BASE (${MEDIAPIPE_WASM_BASE}) must sit under the allowlisted ${MEDIAPIPE_SCOPE}`,
  );
});

test("script-src trusts no bare origins", () => {
  for (const source of sources("script-src")) {
    assert.ok(
      isPinnedScriptSource(source),
      `script-src must stay scoped, found broad source \`${source}\``,
    );
  }
});

test("pinned script source check rejects broad sources", () => {
  // Guards the guard: the check above is only worth having if it fails on the
  // shapes that reopen the allowlist.
  for (const allowed of [
    "'self'",
    "'wasm-unsafe-eval'",
    MEDIAPIPE_SCOPE,
    "https://cdn.jsdelivr.net/npm/@mediapipe/tasks-vision@0.10.35/wasm/vision_wasm_internal.js",
  ]) {
    assert.ok(isPinnedScriptSource(allowed), `${allowed} should pass`);
  }
  for (const rejected of [
    "https://cdn.jsdelivr.net",
    "https://cdn.jsdelivr.net/",
    "https://cdn.jsdelivr.net/npm/",
    "https://cdn.jsdelivr.net/gh/",
    "https://*.jsdelivr.net/npm/@mediapipe/",
    "https:",
    "*",
  ]) {
    assert.ok(
      !isPinnedScriptSource(rejected),
      `${rejected} should be rejected`,
    );
  }
});

test("media directives allow the buzz-media scheme", () => {
  // `rewriteRelayUrl` emits `buzz-media://localhost/...` until the loopback
  // proxy port resolves, so cold-start media renders through the custom
  // scheme (mapped to `http://buzz-media.localhost` on Windows).
  for (const directive of ["img-src", "media-src", "connect-src"]) {
    const allowed = sources(directive);
    assert.ok(
      allowed.includes("buzz-media:"),
      `${directive} must allow buzz-media:`,
    );
    assert.ok(
      allowed.includes("http://buzz-media.localhost"),
      `${directive} must allow http://buzz-media.localhost`,
    );
  }
});

test("connect-src allows IPC and cleartext relays", () => {
  // `ipc:` / `http://ipc.localhost` carry every Tauri command. Cleartext
  // `http:`/`ws:` stay allowed because a relay URL is user-supplied and the
  // app accepts plain `ws://` on any host (`communityStorage::normalizeRelayUrl`,
  // the community edit form): `relayProbe` opens a browser WebSocket to it,
  // so narrowing this to loopback would report reachable relays as dead —
  // while the real connection, which runs through tauri-plugin-websocket in
  // Rust, is not governed by CSP at all. Blanket `https:` is already allowed,
  // so restricting the cleartext schemes would close no exfiltration path.
  const allowed = sources("connect-src");
  for (const source of [
    "ipc:",
    "http://ipc.localhost",
    "https:",
    "http:",
    "wss:",
    "ws:",
  ]) {
    assert.ok(allowed.includes(source), `connect-src must allow ${source}`);
  }
});

test("script-src stays free of unsafe-inline and unsafe-eval", () => {
  const allowed = sources("script-src");
  // The inline boot script in index.html is covered by Tauri's build-time
  // SHA-256 hashing (scripts only — Tauri nonces inline <style> elements via
  // a different path), so neither escape hatch is ever needed here. The boot
  // background style was moved to public/boot.css to avoid the nonce path for
  // style-src; see boot.css for the full rationale.
  assert.ok(!allowed.includes("'unsafe-inline'"));
  assert.ok(!allowed.includes("'unsafe-eval'"));
});
