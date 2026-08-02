/**
 * Unit tests for tauri.ts — focused on `applyTauriRateLimitIfNeeded`, the
 * extracted `relay rate-limited:` classifier that activates the shared
 * rate-limit gate when Rust emits an HTTP 429 error prefix.
 *
 * Testing the exported production function (not a local copy) ensures any
 * change to the classifier logic is immediately covered here.
 */
import assert from "node:assert/strict";
import test from "node:test";

// ── Fake-timer + gate setup ───────────────────────────────────────────────────

let fakeNow = 0;
const pendingTimers = new Map();
let nextTimerId = 1;

function fakeSetTimeout(fn, ms) {
  const id = nextTimerId++;
  pendingTimers.set(id, { fn, fireAt: fakeNow + ms });
  return id;
}

function fakeClearTimeout(id) {
  pendingTimers.delete(id);
}

function tickTo(ms) {
  fakeNow = ms;
  for (const [id, { fn, fireAt }] of Array.from(pendingTimers.entries())) {
    if (fireAt <= fakeNow) {
      pendingTimers.delete(id);
      fn();
    }
  }
}

const origDateNow = Date.now;
function setFakeNow(ms) {
  fakeNow = ms;
  Date.now = () => fakeNow;
}

globalThis.window = {
  setTimeout: fakeSetTimeout,
  clearTimeout: fakeClearTimeout,
};

setFakeNow(0);

const { isRateLimited, resetRateLimitGate } = await import(
  "./relayRateLimitGate.ts"
);

// Import the production classifier from tauri.ts — tests must exercise the
// real function, not a local copy, so a logic change is always caught here.
const { applyTauriRateLimitIfNeeded } = await import("./tauri.ts");

function resetGate(startMs = 0) {
  pendingTimers.clear();
  nextTimerId = 1;
  setFakeNow(startMs);
  resetRateLimitGate();
}

// ── applyTauriRateLimitIfNeeded: relay rate-limited: prefix ───────────────────

test("relay rate-limited: prefix activates the rate-limit gate", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay rate-limited: retry in 10s");
  assert.equal(isRateLimited(), true, "gate must be active after 429 error");
});

test("relay rate-limited: prefix parses the retry hint and arms the gate duration", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay rate-limited: retry in 7s");
  // Gate should be active at 6s.
  setFakeNow(6_000);
  assert.equal(isRateLimited(), true);
  // Gate should expire after 7s.
  tickTo(7_001);
  assert.equal(isRateLimited(), false);
});

test("relay rate-limited: with no hint uses the 10s default", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay rate-limited: quota exceeded");
  tickTo(9_999);
  assert.equal(isRateLimited(), true);
  tickTo(10_001);
  assert.equal(isRateLimited(), false);
});

test("non-rate-limited error does not activate the gate", () => {
  resetGate(0);
  applyTauriRateLimitIfNeeded("relay returned 404 Not Found");
  assert.equal(
    isRateLimited(),
    false,
    "gate must remain inactive for unrelated errors",
  );
});

test("relay rate-limited: prefix check is case-sensitive (Rust always emits lowercase)", () => {
  resetGate(0);
  // The prefix from Rust is always lowercase; mixed-case must not trigger it.
  applyTauriRateLimitIfNeeded("Relay rate-limited: retry in 5s");
  assert.equal(
    isRateLimited(),
    false,
    "uppercase prefix must not activate gate (relay emits lowercase only)",
  );
});

// ── fromRawAcpRuntimeCatalogEntry: custom row API-boundary (B-2) ─────────────
//
// These tests feed real raw custom catalog rows through fromRawAcpRuntimeCatalogEntry
// and verify the Rust→TypeScript mapping boundary: definition_env (snake_case)
// arrives as definitionEnv (camelCase), source "custom" is preserved, and the
// env round-trips end-to-end so a save-then-edit cycle cannot erase env.

const { fromRawAcpRuntimeCatalogEntry } = await import("./tauri.ts");

test("fromRawAcpRuntimeCatalogEntry maps definition_env to definitionEnv", () => {
  const raw = {
    id: "my-harness",
    label: "My Harness",
    availability: "available",
    command: "my-bin",
    source: "custom",
    definition_env: { ANTHROPIC_API_KEY: "sk-test", MODEL: "claude-3" },
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.deepStrictEqual(entry.definitionEnv, {
    ANTHROPIC_API_KEY: "sk-test",
    MODEL: "claude-3",
  });
  assert.equal(entry.source, "custom");
});

test("fromRawAcpRuntimeCatalogEntry defaults definitionEnv to {} when absent", () => {
  // Rust serialization skips empty BTreeMap, so definition_env will be absent
  // for harnesses with no env defined — the mapper must default to {}.
  const raw = {
    id: "no-env-harness",
    label: "No Env",
    availability: "available",
    command: "no-env-bin",
    source: "custom",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.deepStrictEqual(
    entry.definitionEnv,
    {},
    "absent definition_env must map to empty object, not undefined",
  );
});

test("fromRawAcpRuntimeCatalogEntry preserves source preset", () => {
  const raw = {
    id: "cursor",
    label: "Cursor",
    availability: "available",
    command: "cursor",
    source: "preset",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.equal(entry.source, "preset");
  assert.deepStrictEqual(entry.definitionEnv, {});
});

test("fromRawAcpRuntimeCatalogEntry env round-trips through edit payload shape", () => {
  // Simulate the full save → re-open cycle: raw entry comes back from Rust
  // with definition_env populated; the edit form reads entry.definitionEnv.
  // Verify the env values are identical before and after the mapper.
  const envValues = { OPENAI_API_KEY: "sk-live-abc", REGION: "us-east-1" };
  const raw = {
    id: "openai-harness",
    label: "OpenAI",
    availability: "not_installed",
    command: "openai-agent",
    source: "custom",
    definition_env: envValues,
    default_args: ["--acp"],
    can_auto_install: false,
    requires_external_cli: true,
    install_hint: "Install the OpenAI CLI",
    install_instructions_url: "https://platform.openai.com/docs",
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  // The edit form reads entry.definitionEnv; it must equal the original env.
  assert.deepStrictEqual(
    entry.definitionEnv,
    envValues,
    "env must round-trip: edit form must see the same values that Rust serialized",
  );
});

// ── fromRawRemoteHarness: the remote catalog wire boundary ───────────────────
//
// The provider emits `exclusive: true` only for entries that name a persistent
// identity on the host. Absent must stay absent: the desktop reads "no field"
// as "deploy as many as you like", and inventing a `false` would have the app
// asserting something the provider never said.

const { fromRawRemoteHarness } = await import("./tauri.ts");

test("fromRawRemoteHarness carries an asserted exclusive flag", () => {
  const harness = fromRawRemoteHarness({
    id: "hermes-default",
    label: "Hermes (default)",
    command: "hermes",
    args: ["--profile", "default", "acp"],
    available: true,
    binaryPath: "/usr/local/bin/hermes",
    exclusive: true,
  });
  assert.equal(harness.exclusive, true);
  assert.deepStrictEqual(harness.args, ["--profile", "default", "acp"]);
});

test("fromRawRemoteHarness leaves exclusive absent when the provider is silent", () => {
  const harness = fromRawRemoteHarness({
    id: "claude",
    label: "Claude Code",
    command: "claude-code-acp",
    available: true,
  });
  assert.equal(
    Object.hasOwn(harness, "exclusive"),
    false,
    "an absent flag must not become a claim the provider never made",
  );
  // Everything else still degrades to today's defaults.
  assert.deepStrictEqual(harness.args, []);
  assert.deepStrictEqual(harness.env, {});
  assert.equal(harness.binaryPath, null);
  assert.equal(harness.version, null);
});

test("fromRawRemoteHarness treats a false or null exclusive as not exclusive", () => {
  for (const exclusive of [false, null]) {
    const harness = fromRawRemoteHarness({
      id: "codex",
      command: "codex",
      available: true,
      exclusive,
    });
    assert.equal(Object.hasOwn(harness, "exclusive"), false);
    assert.equal(
      harness.label,
      "codex",
      "a missing label falls back to the id",
    );
  }
});

// ── providerRecoveryOf: the actionable half of a provider failure ────────────

const { providerRecoveryOf, TauriInvokeError } = await import("./tauri.ts");

/** A provider command rejection as `invokeTauri` produces it. */
function rejection(payload) {
  return new TauriInvokeError(payload.message ?? "failed", payload);
}

test("providerRecoveryOf reads the recovery off a structured provider failure", () => {
  const recovery = providerRecoveryOf(
    rejection({
      message: "this host requires Tailscale SSH authentication in a browser",
      recovery: {
        action: "open_url",
        url: "https://login.tailscale.com/a/1a2b3c4d",
      },
    }),
  );
  assert.deepEqual(recovery, {
    action: "open_url",
    url: "https://login.tailscale.com/a/1a2b3c4d",
  });
});

test("providerRecoveryOf returns null for an ordinary failure", () => {
  // The overwhelmingly common case: a failure with no recovery renders as its
  // message alone, with no button.
  assert.equal(providerRecoveryOf(rejection({ message: "exit 255" })), null);
  assert.equal(providerRecoveryOf(new Error("ssh failed")), null);
  assert.equal(providerRecoveryOf("a bare string"), null);
  assert.equal(providerRecoveryOf(null), null);
  assert.equal(providerRecoveryOf(undefined), null);
});

test("providerRecoveryOf ignores a malformed or unknown recovery", () => {
  for (const recovery of [
    null,
    "https://login.tailscale.com/a/tok",
    { action: "run_command", command: "rm -rf /" },
    { action: "open_url" },
    { action: "open_url", url: "" },
    { action: "open_url", url: 42 },
  ]) {
    assert.equal(
      providerRecoveryOf(rejection({ message: "failed", recovery })),
      null,
      JSON.stringify(recovery),
    );
  }
});

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
