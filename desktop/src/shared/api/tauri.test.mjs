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

// Import production API functions so tests exercise the real invoke payload
// construction rather than a duplicated frontend shape.
const { applyTauriRateLimitIfNeeded, createManagedAgent } = await import(
  "./tauri.ts"
);

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

test("fromRawAcpRuntimeCatalogEntry maps generic readiness and account actions", () => {
  const entry = fromRawAcpRuntimeCatalogEntry({
    id: "ompk",
    label: "Oh My Pi",
    availability: "available",
    command: "ompk",
    source: "preset",
    default_args: ["acp"],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
    auth_status: "unknown",
    runtime_readiness: "model_unavailable",
    can_connect_account: true,
  });

  assert.equal(entry.runtimeReadiness, "model_unavailable");
  assert.equal(entry.canConnectAccount, true);
});

test("fromRawAcpRuntimeCatalogEntry maps setup metadata without values", () => {
  const entry = fromRawAcpRuntimeCatalogEntry({
    id: "remote-agent-computer",
    label: "Remote agent computer",
    availability: "available",
    command: "buzz-manual-agent-acp",
    source: "builtin",
    default_args: [],
    can_auto_install: false,
    install_hint: "",
    install_instructions_url: "",
    runtime_readiness: "configuration_required",
    setup_fields: [
      {
        env_key: "MANUAL_AGENT_TOKEN",
        label: "Manual Agent app password",
        placeholder: "Paste the app password",
        helper_text: "Local secret",
        kind: "secret",
        validation: "app_password",
        required: true,
      },
    ],
  });
  assert.deepEqual(entry.setupFields, [
    {
      envKey: "MANUAL_AGENT_TOKEN",
      label: "Manual Agent app password",
      placeholder: "Paste the app password",
      helperText: "Local secret",
      kind: "secret",
      validation: "app_password",
      required: true,
    },
  ]);
  assert.equal(JSON.stringify(entry).includes("test-app-password"), false);
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

// ── max_parallelism → maxParallelism mapping ──────────────────────────────────

test("fromRawAcpRuntimeCatalogEntry maps max_parallelism to maxParallelism when present", () => {
  const raw = {
    id: "openclaw",
    label: "OpenClaw",
    availability: "not_installed",
    command: null,
    source: "preset",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
    max_parallelism: 5,
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.equal(
    entry.maxParallelism,
    5,
    "max_parallelism: 5 must map to maxParallelism: 5",
  );
});

test("fromRawAcpRuntimeCatalogEntry omits maxParallelism when max_parallelism is absent", () => {
  const raw = {
    id: "goose",
    label: "Goose",
    availability: "available",
    command: "goose",
    source: "builtin",
    default_args: [],
    can_auto_install: false,
    requires_external_cli: false,
    install_hint: "",
    install_instructions_url: "",
    // No max_parallelism field — uncapped harness.
  };
  const entry = fromRawAcpRuntimeCatalogEntry(raw);
  assert.equal(
    entry.maxParallelism,
    undefined,
    "uncapped harness must have maxParallelism: undefined",
  );
});

// ── createManagedAgent runtime identity forwarding ───────────────────────────

test("createManagedAgent forwards catalog identity with live-default args", async () => {
  let captured = null;
  const previousInternals = window.__TAURI_INTERNALS__;
  window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      captured = { command, args };
      throw new Error("captured create payload");
    },
  };

  try {
    await assert.rejects(
      createManagedAgent({
        name: "Fizz",
        runtimeId: "ompk",
        agentCommand: "ompk",
        agentArgs: [],
        harnessOverride: false,
      }),
      /captured create payload/,
    );
  } finally {
    if (previousInternals === undefined) {
      delete window.__TAURI_INTERNALS__;
    } else {
      window.__TAURI_INTERNALS__ = previousInternals;
    }
  }

  assert.equal(captured.command, "create_managed_agent");
  assert.equal(captured.args.input.runtimeId, "ompk");
  assert.deepEqual(captured.args.input.agentArgs, []);
  assert.equal(captured.args.input.harnessOverride, false);
});

// ── Teardown ──────────────────────────────────────────────────────────────────

test("teardown — restore Date.now", () => {
  Date.now = origDateNow;
  assert.ok(true);
});
