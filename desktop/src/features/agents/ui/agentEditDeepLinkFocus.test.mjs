/**
 * Regression tests for useAgentEditDeepLinkFocus — the two deep-link focus
 * blockers that failed scenario 8, exercised against the real hook.
 *
 * Blocker 2 (model): the provider/model combobox trigger is `disabled` while
 * model discovery is in flight. The one-shot focus effect must NOT latch or
 * fire against a disabled control; it must retry and land focus once discovery
 * resolves (modelDiscoveryLoading flips false and the control loses `disabled`).
 *
 * Blocker 1 (env_key): env_key targets live inside the Advanced accordion,
 * which seeds collapsed. The hook must expand Advanced (setShowAdvancedFields)
 * when an env_key target is requested so EnvVarsEditor can mount and focus.
 *
 * Autofocus handler: with a normalized target that is not yet focusable, the
 * returned onOpenAutoFocus must NOT preventDefault (so Radix supplies a baseline
 * and focus is never stranded on <body>); once the target is focusable it must
 * preventDefault and claim focus.
 */

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  Node: dom.window.Node,
  IS_REACT_ACT_ENVIRONMENT: true,
  window: dom.window,
});
// Real rAF (macrotask-backed): the focus effect schedules the actual focus()
// call in a rAF, so a no-op rAF would swallow it.
globalThis.requestAnimationFrame = (cb) => setTimeout(() => cb(Date.now()), 0);
globalThis.cancelAnimationFrame = (id) => clearTimeout(id);

/** Insert a focusable control with the given id; optionally disabled. */
function mountTarget(id, { disabled = false } = {}) {
  const el = dom.window.document.createElement("button");
  el.id = id;
  if (disabled) el.setAttribute("disabled", "");
  dom.window.document.body.appendChild(el);
  let focusCalls = 0;
  el.focus = () => {
    focusCalls++;
  };
  el.scrollIntoView = () => {};
  return {
    el,
    get focusCalls() {
      return focusCalls;
    },
  };
}

afterEach(() => {
  dom.window.document.body.innerHTML = "";
});

async function importHook() {
  return import("./useAgentEditDeepLinkFocus.ts");
}

async function flushRaf(act) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 5));
  });
}

test("model target stays unfocused while disabled, then focuses once discovery resolves", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useAgentEditDeepLinkFocus } = await importHook();
  const model = mountTarget("edit-agent-model", { disabled: true });

  try {
    const { rerender } = renderHook(
      (props) => useAgentEditDeepLinkFocus(props),
      {
        initialProps: {
          open: true,
          initialFocus: { type: "normalized_field", field: "model" },
          contextKey: "pk:",
          llmProviderFieldVisible: true,
          modelDiscoveryLoading: true,
          setShowAdvancedFields: () => {},
        },
      },
    );
    await flushRaf(act);
    assert.equal(
      model.focusCalls,
      0,
      "must not focus the model combobox while it is disabled during discovery",
    );

    // Discovery resolves: control becomes focusable and the loading signal flips.
    model.el.removeAttribute("disabled");
    rerender({
      open: true,
      initialFocus: { type: "normalized_field", field: "model" },
      contextKey: "pk:",
      llmProviderFieldVisible: true,
      modelDiscoveryLoading: false,
      setShowAdvancedFields: () => {},
    });
    await flushRaf(act);
    assert.equal(
      model.focusCalls,
      1,
      "must focus the model combobox exactly once after discovery resolves",
    );
  } finally {
    cleanup();
  }
});

test("env_key target expands the Advanced accordion", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useAgentEditDeepLinkFocus } = await importHook();
  const expanded = [];

  try {
    renderHook(() =>
      useAgentEditDeepLinkFocus({
        open: true,
        initialFocus: { type: "env_key", key: "ANTHROPIC_API_KEY" },
        contextKey: "pk:",
        llmProviderFieldVisible: true,
        modelDiscoveryLoading: false,
        setShowAdvancedFields: (next) => expanded.push(next),
      }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    assert.deepEqual(
      expanded,
      [true],
      "env_key target must expand Advanced so EnvVarsEditor mounts",
    );
  } finally {
    cleanup();
  }
});

test("onOpenAutoFocus defers to Radix while the target is disabled and claims it once focusable", async () => {
  const { cleanup, renderHook } = await import("@testing-library/react");
  const { useAgentEditDeepLinkFocus } = await importHook();
  const model = mountTarget("edit-agent-model", { disabled: true });

  try {
    const { result } = renderHook(() =>
      useAgentEditDeepLinkFocus({
        open: true,
        initialFocus: { type: "normalized_field", field: "model" },
        contextKey: "pk:",
        llmProviderFieldVisible: true,
        modelDiscoveryLoading: true,
        setShowAdvancedFields: () => {},
      }),
    );

    let prevented = false;
    const disabledEvent = { preventDefault: () => (prevented = true) };
    result.current.onOpenAutoFocus(disabledEvent);
    assert.equal(
      prevented,
      false,
      "must not suppress Radix autofocus while the target is disabled (else focus strands on <body>)",
    );
    assert.equal(model.focusCalls, 0, "must not focus a disabled target");

    model.el.removeAttribute("disabled");
    prevented = false;
    result.current.onOpenAutoFocus({
      preventDefault: () => (prevented = true),
    });
    assert.equal(
      prevented,
      true,
      "must suppress Radix autofocus once the target is focusable",
    );
    assert.equal(
      model.focusCalls,
      1,
      "must claim focus on the focusable target",
    );
  } finally {
    cleanup();
  }
});
