/**
 * D-section field-parity mount tests.
 *
 * PR #5328 merges main's two edit dialogs (AgentDefinitionDialog +
 * AgentInstanceEditDialog) into one AgentEditMergedDialog. Four D-section
 * (definition-field) capabilities from main's PersonaAdvancedFields /
 * AgentDefinitionDialog edit mode were dropped in the merge and restored here:
 *
 *   Fix2  parallelism cap hint (amber "runs at most N" warning)
 *   Fix4  provider API-key field (custom provider + top-level secret)
 *   ----  descriptor-driven numeric tuning knobs + buzz-agent effort knob
 *   ----  env-editor structured-key hiding + mint-key annotation
 *
 * These mount the REAL dialog in a definition-only context (react-dom + jsdom)
 * so the restored controls are genuinely rendered, the seed/derive effects run,
 * and the D-section reads the definition runtime/provider/env — not the
 * instance overlay. The Tauri boundary is stubbed at
 * window.__TAURI_INTERNALS__.invoke; the query cache is pre-seeded so the
 * dialog paints on first settle without live IPC.
 *
 * Emit-layer routing (a D-env edit lands in personaInput, never agentInput) is
 * covered by the pure-logic tests in agentFormModel.test.mjs; these assert the
 * rendered surface.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** QueryClients created per test, cleared in afterEach so no gc timer lingers. */
const liveClients = [];
/**
 * Radix's dismissable-layer / focus-scope schedule global setTimeout/setInterval
 * that outlive the render. jsdom's own timers delegate to the global, so we
 * track every timer we hand out and clear the survivors in teardown — otherwise
 * the process hangs after the tests pass.
 */
const pendingTimers = new Set();
const nativeSetTimeout = globalThis.setTimeout;
const nativeSetInterval = globalThis.setInterval;
const nativeClearTimeout = globalThis.clearTimeout;
const nativeClearInterval = globalThis.clearInterval;

before(() => {
  globalThis.setTimeout = (fn, ms, ...args) => {
    const id = nativeSetTimeout(
      (...a) => {
        pendingTimers.delete(id);
        return fn(...a);
      },
      ms,
      ...args,
    );
    pendingTimers.add(id);
    return id;
  };
  globalThis.setInterval = (fn, ms, ...args) => {
    const id = nativeSetInterval(fn, ms, ...args);
    pendingTimers.add(id);
    return id;
  };
  globalThis.clearTimeout = (id) => {
    pendingTimers.delete(id);
    return nativeClearTimeout(id);
  };
  globalThis.clearInterval = (id) => {
    pendingTimers.delete(id);
    return nativeClearInterval(id);
  };

  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (key in globalThis) continue;
    try {
      globalThis[key] = dom.window[key];
    } catch {
      /* getter-only global — skip */
    }
  }
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
  });
  for (const key of [
    "Event",
    "CustomEvent",
    "MouseEvent",
    "KeyboardEvent",
    "FocusEvent",
    "PointerEvent",
    "InputEvent",
    "UIEvent",
  ]) {
    if (dom.window[key]) globalThis[key] = dom.window[key];
  }
  dom.window.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  globalThis.ResizeObserver = dom.window.ResizeObserver;
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  dom.window.HTMLElement.prototype.hasPointerCapture = () => false;
  dom.window.HTMLElement.prototype.releasePointerCapture = () => {};
  dom.window.HTMLElement.prototype.setPointerCapture = () => {};
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  for (const qc of liveClients.splice(0)) {
    qc.clear();
    qc.unmount();
  }
  for (const id of pendingTimers) nativeClearTimeout(id);
  pendingTimers.clear();
});

after(() => {
  globalThis.setTimeout = nativeSetTimeout;
  globalThis.setInterval = nativeSetInterval;
  globalThis.clearTimeout = nativeClearTimeout;
  globalThis.clearInterval = nativeClearInterval;
  dom.window.close();
});

/**
 * A buzz-agent runtime catalog entry with the numeric + thinking env vars set
 * (so descriptors derive) and a parallelism cap (so the cap hint can fire).
 *
 * `availability` is deliberately NOT "available": an available runtime is the
 * sole trigger for the asynchronous model-discovery IPC, whose settle resolves
 * after the mount's act window and logs act(...) warnings. Model discovery is
 * orthogonal to D-section field parity — every field under test (tuning knobs,
 * effort, API key, cap hint, env-key hiding) derives from the catalog entry and
 * provider, not from availability, exactly as main's PersonaAdvancedFields does.
 * Suppressing discovery keeps these mounts deterministic without weakening the
 * parity claim.
 */
const BUZZ_AGENT_RUNTIME = {
  id: "buzz-agent",
  label: "Buzz Agent",
  avatarUrl: "",
  availability: "not_installed",
  command: "buzz-agent",
  binaryPath: "buzz-agent",
  defaultArgs: [],
  mcpCommand: null,
  modelEnvVar: "BUZZ_AGENT_MODEL",
  providerEnvVar: "BUZZ_AGENT_PROVIDER",
  thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
  maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
  contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
  maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
  installHint: "",
  installInstructionsUrl: "",
  canAutoInstall: false,
  requiresExternalCli: false,
  underlyingCliPath: null,
  nodeRequired: false,
  authStatus: { status: "not_applicable" },
  loginHint: null,
  source: "builtin",
  maxParallelism: 2,
};

/**
 * Build a definition-only edit context. `envVars`, `parallelism`, and
 * `provider` default to values that exercise the restored controls.
 */
function definitionCtx(overrides = {}) {
  return {
    kind: "definition-only",
    definition: {
      id: "def-parity",
      displayName: "Brain",
      avatarUrl: null,
      systemPrompt: "You are helpful.",
      runtime: "buzz-agent",
      model: "claude-sonnet-4-5",
      provider: "anthropic",
      namePool: [],
      isBuiltIn: false,
      isActive: true,
      shared: false,
      sourceTeam: null,
      catalogSource: null,
      envVars: {},
      respondTo: null,
      respondToAllowlist: [],
      parallelism: 8,
      createdAt: "2025-01-01T00:00:00Z",
      updatedAt: "2025-01-01T00:00:00Z",
      ...overrides,
    },
  };
}

/** Seed a QueryClient with every catalog the dialog reads on open. */
async function makeSeededClient() {
  const { QueryClient } = await import("@tanstack/react-query");
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Number.POSITIVE_INFINITY },
    },
  });
  qc.setQueryData(["acp-runtimes"], [BUZZ_AGENT_RUNTIME]);
  qc.setQueryData(["personas"], []);
  qc.setQueryData(["managed-agents"], []);
  qc.setQueryData(["teams"], []);
  qc.setQueryData(["baked-build-env-keys"], []);
  qc.setQueryData(["agent-access-owner-only"], false);
  qc.setQueryData(["globalAgentConfig"], {
    env_vars: {},
    provider: null,
    model: null,
    preferred_runtime: null,
  });
  qc.setQueryData(["runtime-file-config", "buzz-agent"], {
    provider: null,
    model: null,
    satisfiedEnvKeys: [],
  });
  liveClients.push(qc);
  return qc;
}

/** Stub the Tauri boundary — the dialog only reads; nothing here mutates. */
function installInvoke() {
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd) => {
      if (cmd === "list_personas") return [];
      if (cmd === "list_managed_agents") return [];
      if (cmd === "get_runtime_file_config") {
        return { provider: null, model: null, satisfiedEnvKeys: [] };
      }
      return null;
    },
    transformCallback: (cb) => cb,
    unregisterCallback() {},
    convertFileSrc: (p) => p,
  };
}

async function renderDialog(qc, ctx) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { ThemeProvider } = await import("@/shared/theme/ThemeProvider");
  const { AgentEditMergedDialog } = await import("./AgentEditMergedDialog.tsx");

  return render(
    createElement(
      QueryClientProvider,
      { client: qc },
      createElement(
        ThemeProvider,
        { defaultTheme: "buzz" },
        createElement(AgentEditMergedDialog, {
          open: true,
          onOpenChange: () => {},
          ctx,
        }),
      ),
    ),
  );
}

async function settle(ms = 40) {
  const { act } = await import("react");
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

/** Click the D-section "Advanced" disclosure to reveal env + tuning fields. */
async function openAdvanced() {
  const { fireEvent, screen, act } = await import("@testing-library/react");
  const reactAct = (await import("react")).act;
  const advanced = screen.getByRole("button", { name: "Advanced" });
  await reactAct(async () => {
    fireEvent.click(advanced);
    await new Promise((r) => setTimeout(r, 30));
  });
  void act;
}

// ── Fix4: provider API-key field in a definition-only edit ───────────────────

test("test_definition_only_dsection_renders_provider_api_key_field_for_custom_provider", async () => {
  // Main's AgentDefinitionDialog edit mode renders PersonaProviderApiKeyField
  // when the provider is custom and exposes a top-level secret. A definition
  // whose provider is anthropic must therefore expose the ANTHROPIC_API_KEY
  // field in the merged D-section — the field the merge dropped from def-only.
  const qc = await makeSeededClient();
  installInvoke();
  await renderDialog(qc, definitionCtx());
  await settle();

  const { screen } = await import("@testing-library/react");
  const apiKey = screen.getByTestId("persona-provider-api-key");
  assert.ok(
    apiKey,
    "the provider API-key input must render for a custom provider",
  );

  // The env var name is shown as a monospace hint, and the label is provider-derived.
  assert.match(
    dom.window.document.body.textContent ?? "",
    /ANTHROPIC_API_KEY/,
    "the API-key field must show its ANTHROPIC_API_KEY env-var hint",
  );
  assert.match(
    dom.window.document.body.textContent ?? "",
    /Anthropic API Key/,
    "the API-key field label must be the provider-specific label",
  );
});

test("test_definition_only_dsection_hides_provider_api_key_field_when_provider_has_no_secret", async () => {
  // Databricks uses OAuth PKCE — no typed secret credential. Main renders no
  // API-key field for it, so the merged D-section must not either (guards
  // against adding a field main never had).
  const qc = await makeSeededClient();
  installInvoke();
  await renderDialog(
    qc,
    definitionCtx({ provider: "databricks", model: null }),
  );
  await settle();

  const { screen } = await import("@testing-library/react");
  assert.equal(
    screen.queryByTestId("persona-provider-api-key"),
    null,
    "a provider with no typed secret must not render an API-key field",
  );
});

// ── Fix2: parallelism cap hint ───────────────────────────────────────────────

test("test_definition_only_dsection_shows_parallelism_cap_hint_when_requested_exceeds_runtime_cap", async () => {
  // The definition requests parallelism 8; the buzz-agent runtime caps at 2.
  // Main's PersonaAdvancedFields shows an amber hint explaining the agent will
  // run at the cap. The merged D-section must reproduce it without clamping.
  const qc = await makeSeededClient();
  installInvoke();
  await renderDialog(qc, definitionCtx({ parallelism: 8 }));
  await settle();

  assert.match(
    dom.window.document.body.textContent ?? "",
    /runs at most 2 parallel conversations/,
    "the amber cap hint must appear when requested parallelism exceeds the runtime cap",
  );
});

test("test_definition_only_dsection_hides_parallelism_cap_hint_when_within_cap", async () => {
  // At-or-below the cap, no hint — the value is honored as requested.
  const qc = await makeSeededClient();
  installInvoke();
  await renderDialog(qc, definitionCtx({ parallelism: 1 }));
  await settle();

  assert.doesNotMatch(
    dom.window.document.body.textContent ?? "",
    /runs at most/,
    "no cap hint when the requested parallelism is within the runtime cap",
  );
});

// ── Numeric tuning + effort knobs (restored from PersonaAdvancedFields) ──────

test("test_definition_only_dsection_advanced_renders_numeric_and_effort_tuning_knobs", async () => {
  // buzz-agent exposes three numeric env-var fields plus the thinking-effort
  // knob. All four render under the D-section Advanced disclosure, reading the
  // definition env layer — a value already set on the definition shows in its
  // input rather than as a generic env row.
  const qc = await makeSeededClient();
  installInvoke();
  await renderDialog(
    qc,
    definitionCtx({ envVars: { BUZZ_AGENT_MAX_OUTPUT_TOKENS: "4096" } }),
  );
  await settle();
  await openAdvanced();

  const { screen } = await import("@testing-library/react");
  const maxTokens = screen.getByTestId("numeric-max-output-tokens-input");
  assert.ok(maxTokens, "max output tokens input must render");
  assert.equal(
    maxTokens.value,
    "4096",
    "the numeric input must read the definition's stored env value",
  );
  assert.ok(
    screen.getByTestId("numeric-context-limit-input"),
    "context limit input must render",
  );
  assert.ok(
    screen.getByTestId("numeric-max-rounds-input"),
    "max rounds input must render",
  );
  assert.ok(
    screen.getByTestId("ba-thinking-effort-select"),
    "the buzz-agent thinking-effort knob must render",
  );
});

// ── Env editor: structured-key hiding + mint-key annotation ──────────────────

test("test_definition_only_dsection_env_editor_hides_structured_keys_and_annotates_mint_key", async () => {
  // The D-section env editor must hide keys owned by structured controls (the
  // provider secret, the numeric descriptors, the effort key) so they are never
  // double-edited, while a plain user key stays a generic row and OPENAI_API_KEY
  // carries its mint-key annotation.
  const qc = await makeSeededClient();
  installInvoke();
  await renderDialog(
    qc,
    definitionCtx({
      envVars: {
        BUZZ_AGENT_MAX_OUTPUT_TOKENS: "4096", // structured → hidden
        ANTHROPIC_API_KEY: "sk-live", // secret → hidden (renders as API-key field)
        OPENAI_API_KEY: "sk-openai", // annotated generic row
        MY_PLAIN_VAR: "hello", // plain generic row
      },
    }),
  );
  await settle();
  await openAdvanced();

  const { screen } = await import("@testing-library/react");
  const genericKeys = screen
    .queryAllByTestId("env-vars-key")
    .map((el) => el.value);

  assert.ok(
    genericKeys.includes("MY_PLAIN_VAR"),
    "a plain env key must remain an editable generic row",
  );
  assert.ok(
    !genericKeys.includes("BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
    "a numeric structured key must be hidden from the generic env rows",
  );
  assert.ok(
    !genericKeys.includes("ANTHROPIC_API_KEY"),
    "the provider secret must be hidden from the generic env rows",
  );
  assert.ok(
    genericKeys.includes("OPENAI_API_KEY"),
    "OPENAI_API_KEY is not structured for this provider — stays a generic row",
  );
  const annotations = screen
    .queryAllByTestId("env-vars-key-annotation")
    .map((el) => el.textContent ?? "");
  assert.ok(
    annotations.some((t) => /minting agent trading cards/.test(t)),
    "the OPENAI_API_KEY mint-key annotation must render",
  );
});
