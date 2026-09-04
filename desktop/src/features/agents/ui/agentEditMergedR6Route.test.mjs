/**
 * R6 route-level behavioral test — owner-review of an agent-origin update.
 *
 * The R6 dispatch requires evidence that a respond-to-only change requested by
 * an agent travels the full production chain and is not hidden state:
 *
 *   reviewOverridesForUpdate(request)   ← the useAgentManagement mapping layer
 *     → AgentEditMergedDialog initialValueOverrides
 *       → the RENDERED "Who can send instructions" control
 *         → onValidate permission gate (blocks a denied origin)
 *           → runAgentSaveCoordinator → update_persona (behavior.respondTo)
 *
 * The real dialog is mounted (react-dom + jsdom), so the control is genuinely
 * rendered, the seed/override effects actually run, and Save routes through the
 * production submit hook and coordinator. The Tauri boundary is stubbed at
 * window.__TAURI_INTERNALS__.invoke; the settlement re-fetch reads the query
 * cache, which the test updates to mirror a persisted write.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** Records every Tauri command the dialog invokes. */
let invokeCalls = [];
/** QueryClients created per test, cleared in afterEach so no gc timer lingers. */
const liveClients = [];
/**
 * Radix's dismissable-layer / focus-scope schedule global setTimeout/setInterval
 * that outlive the render. They are node-native (jsdom's own timers delegate to
 * the global, so we can't swap the global for jsdom's without infinite
 * recursion), and dom.window.close() cannot reap them — the process hangs after
 * the tests pass. Track every timer we hand out and clear the survivors in
 * teardown.
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

  // Install jsdom's globals so react-dom and Radix find real DOM APIs.
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
  // jsdom's dispatchEvent rejects node-native Event instances — force jsdom's.
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
  // Reap any Radix timers still outstanding after unmount so they don't hold
  // the event loop open.
  for (const id of pendingTimers) nativeClearTimeout(id);
  pendingTimers.clear();
  invokeCalls = [];
});

after(() => {
  globalThis.setTimeout = nativeSetTimeout;
  globalThis.setInterval = nativeSetInterval;
  globalThis.clearTimeout = nativeClearTimeout;
  globalThis.clearInterval = nativeClearInterval;
  dom.window.close();
});

const DEFINITION = {
  id: "def-r6",
  displayName: "Brain",
  avatarUrl: null,
  systemPrompt: "You are helpful.",
  runtime: null,
  model: null,
  provider: null,
  namePool: [],
  isBuiltIn: false,
  isActive: true,
  shared: false,
  sourceTeam: null,
  catalogSource: null,
  envVars: {},
  respondTo: null,
  respondToAllowlist: [],
  parallelism: null,
  createdAt: "2025-01-01T00:00:00Z",
  updatedAt: "2025-01-01T00:00:00Z",
};

/** Seed a QueryClient with the empty catalogs the dialog reads on open. */
async function makeSeededClient() {
  const { QueryClient } = await import("@tanstack/react-query");
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Infinity },
    },
  });
  qc.setQueryData(["acp-runtimes"], []);
  qc.setQueryData(["personas"], [DEFINITION]);
  qc.setQueryData(["managed-agents"], []);
  qc.setQueryData(["teams"], []);
  qc.setQueryData(["baked-build-env-keys"], []);
  qc.setQueryData(["agent-access-owner-only"], false);
  liveClients.push(qc);
  return qc;
}

/**
 * Install a Tauri invoke stub. `onUpdatePersona` is called with the raw
 * update_persona payload and returns the raw persona the backend would persist.
 * That persona becomes the new list_personas result AND is written into the
 * query cache, so the coordinator's settlement re-fetch (which re-runs
 * list_personas) observes the persisted write.
 */
function installInvoke(onUpdatePersona) {
  let currentRaw = DEFINITION_RAW();
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (cmd, args) => {
      invokeCalls.push({ cmd, args });
      if (cmd === "list_personas") return [currentRaw];
      if (cmd === "list_managed_agents") return [];
      if (cmd === "update_persona") {
        currentRaw = onUpdatePersona(args?.input);
        return currentRaw;
      }
      return null;
    },
    transformCallback: (cb) => cb,
    unregisterCallback() {},
    convertFileSrc: (p) => p,
  };
}

function DEFINITION_RAW() {
  return {
    id: DEFINITION.id,
    display_name: DEFINITION.displayName,
    avatar_url: null,
    system_prompt: DEFINITION.systemPrompt,
    runtime: null,
    model: null,
    provider: null,
    name_pool: [],
    is_builtin: false,
    is_active: true,
    shared: false,
    source_team: null,
    env_vars: {},
    respond_to: null,
    respond_to_allowlist: [],
    parallelism: null,
    created_at: DEFINITION.createdAt,
    updated_at: DEFINITION.updatedAt,
  };
}

async function renderR6Dialog(qc, { onValidate, initialValueOverrides }) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClientProvider } = await import("@tanstack/react-query");
  const { ThemeProvider } = await import("@/shared/theme/ThemeProvider");
  const { AgentEditMergedDialog } = await import("./AgentEditMergedDialog.tsx");

  const closed = { value: false };
  const result = render(
    createElement(
      QueryClientProvider,
      { client: qc },
      createElement(
        ThemeProvider,
        { defaultTheme: "buzz" },
        createElement(AgentEditMergedDialog, {
          open: true,
          onOpenChange: (open) => {
            if (!open) closed.value = true;
          },
          ctx: { kind: "definition-only", definition: DEFINITION },
          onValidate,
          initialValueOverrides,
        }),
      ),
    ),
  );
  return { ...result, closed };
}

async function settle(ms = 30) {
  const { act } = await import("react");
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

// ── The requested access change is rendered, not hidden ──────────────────────

test("test_r6_requested_respondTo_is_rendered_on_the_access_control", async () => {
  const { reviewOverridesForUpdate } = await import("../agentManagement.ts");
  // The agent requested public access; this is exactly the field R6 exists to
  // review, so it must be visible with the requested value — not hidden state.
  const overrides = reviewOverridesForUpdate({
    channelId: "chan-1",
    agentName: "Brain",
    respondTo: "anyone",
  });
  assert.deepEqual(
    overrides,
    { respondTo: "anyone" },
    "the mapping layer must carry the requested respondTo and nothing else",
  );

  const qc = await makeSeededClient();
  installInvoke(() => DEFINITION_RAW());
  await renderR6Dialog(qc, {
    onValidate: () => null,
    initialValueOverrides: overrides,
  });
  await settle();

  const trigger = dom.window.document.getElementById("agent-respond-to");
  assert.ok(trigger, "the access control must be rendered in review mode");
  assert.match(
    trigger.textContent ?? "",
    /Anyone/,
    "the rendered control must show the agent-requested 'Anyone', not the owner-only default",
  );
});

// ── onValidate gates the save ────────────────────────────────────────────────

test("test_r6_denied_origin_blocks_save_before_any_write", async () => {
  const qc = await makeSeededClient();
  installInvoke(() => {
    throw new Error("update_persona must not be called when origin is denied");
  });
  const overrides = { respondTo: "anyone" };
  await renderR6Dialog(qc, {
    onValidate: () => "This agent may not act from that channel.",
    initialValueOverrides: overrides,
  });
  await settle();

  const { fireEvent, screen } = await import("@testing-library/react");
  const save = screen.getByTestId("edit-agent-dialog-submit");
  await (await import("react")).act(async () => {
    fireEvent.click(save);
    await new Promise((r) => setTimeout(r, 30));
  });

  assert.equal(
    invokeCalls.some((c) => c.cmd === "update_persona"),
    false,
    "a denied onValidate must block the write entirely",
  );
});

// ── approved review delivers the requested respondTo through the coordinator ──

test("test_r6_approved_review_delivers_requested_respondTo_to_update_persona", async () => {
  const qc = await makeSeededClient();
  let capturedInput = null;
  installInvoke((input) => {
    capturedInput = input;
    // Backend persists the requested access change.
    return { ...DEFINITION_RAW(), respond_to: "anyone" };
  });
  const overrides = { respondTo: "anyone" };
  const { closed } = await renderR6Dialog(qc, {
    onValidate: () => null,
    initialValueOverrides: overrides,
  });
  await settle();

  const { fireEvent, screen } = await import("@testing-library/react");
  const save = screen.getByTestId("edit-agent-dialog-submit");
  await (await import("react")).act(async () => {
    fireEvent.click(save);
    await new Promise((r) => setTimeout(r, 60));
  });

  assert.ok(capturedInput, "update_persona must be called on an approved save");
  assert.equal(
    capturedInput.behavior?.respondTo,
    "anyone",
    "the coordinator must deliver the agent-requested respondTo",
  );
  assert.equal(
    closed.value,
    true,
    "a persisted save must close the dialog (onDone → onOpenChange(false))",
  );
});
