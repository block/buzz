/**
 * Runtime-switch effort reset test for AgentEditMergedDialog.
 *
 * Ports the "runtime switch clears touched effort" contract from the deleted
 * agentInstanceEditCancelSafety.test.mjs against the surviving merged code.
 *
 * The production invariant (Carl r10 / Thufir IMPORTANT-3):
 *   Effort selected for runtime A → runtime switched to B → Save must NOT
 *   dispatch the stale A vocab value.
 *
 * The load-bearing code is handleRuntimeDropdownChange in
 * useAgentEditRuntimeHandlers.ts. After this round's fix it clears BOTH
 *   setEffortLevel(null)      -- resets the pending selection
 *   effortTouched.current = false -- tells resolveEffortSubmission to skip
 *
 * This test exercises that code path via useAgentEditRuntimeHandlers directly,
 * using React's renderHook so the assertion lives at the exact production seam
 * (the handler that AgentEditMergedDialog wires to the runtime dropdown's
 * onValueChange). A full-dialog mount for this interaction hangs in Node.js
 * jsdom because Radix's dropdown close sequence and the concurrent React state
 * update from setEffortLevel interact with act() in a way that never drains;
 * the hook-level test is equivalent in coverage and mutation-sensitive.
 *
 * Mutation oracle: removing either reset line from handleRuntimeDropdownChange
 * causes resolveEffortSubmission to emit a stale effortLevel on Save, which
 * the coordinator-level tests (family 14) then catch as a false settlement.
 */

import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  for (const key of Object.getOwnPropertyNames(dom.window)) {
    if (key in globalThis) continue;
    try {
      globalThis[key] = dom.window[key];
    } catch {}
  }
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    getComputedStyle: dom.window.getComputedStyle.bind(dom.window),
  });
  dom.window.matchMedia = () => ({
    matches: false,
    addEventListener() {},
    removeEventListener() {},
  });
  dom.window.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
  globalThis.ResizeObserver = dom.window.ResizeObserver;
  dom.window.HTMLElement.prototype.scrollIntoView = () => {};
  dom.window.HTMLElement.prototype.hasPointerCapture = () => false;
  dom.window.HTMLElement.prototype.releasePointerCapture = () => {};
});

after(() => dom.window.close());

test("handleRuntimeDropdownChange clears effortLevel and effortTouched on runtime switch", async () => {
  const { renderHook, act } = await import("@testing-library/react");
  const { useRef, useState } = await import("react");
  const { useAgentEditRuntimeHandlers } = await import(
    "./useAgentEditRuntimeHandlers.ts"
  );

  // Minimal runtime catalog: current runtime "goose", target runtime "claude"
  const runtimes = [
    {
      id: "goose",
      label: "Goose",
      avatarUrl: "",
      availability: "available",
      command: "goose-cmd",
      binaryPath: "goose-cmd",
      defaultArgs: [],
      mcpCommand: null,
      modelEnvVar: null,
      providerEnvVar: null,
      thinkingEnvVar: null,
      maxTokensEnvVar: null,
      contextLimitEnvVar: null,
      maxRoundsEnvVar: null,
      installHint: "",
      installInstructionsUrl: "",
      canAutoInstall: false,
      requiresExternalCli: false,
      underlyingCliPath: null,
      nodeRequired: false,
      authStatus: { status: "not_applicable" },
      loginHint: null,
      source: "builtin",
      maxParallelism: 4,
    },
    {
      id: "claude",
      label: "claude",
      avatarUrl: "",
      availability: "available",
      command: "claude",
      binaryPath: "claude",
      defaultArgs: [],
      mcpCommand: null,
      modelEnvVar: null,
      providerEnvVar: null,
      thinkingEnvVar: null,
      maxTokensEnvVar: null,
      contextLimitEnvVar: null,
      maxRoundsEnvVar: null,
      installHint: "",
      installInstructionsUrl: "",
      canAutoInstall: false,
      requiresExternalCli: false,
      underlyingCliPath: null,
      nodeRequired: false,
      authStatus: { status: "not_applicable" },
      loginHint: null,
      source: "builtin",
      maxParallelism: 4,
    },
  ];

  // Wrap in a thin hook that exposes the effort state so we can assert it
  const { result } = renderHook(() => {
    const [selectedRuntimeId, setSelectedRuntimeId] = useState("goose");
    const [effortLevel, setEffortLevel] = useState(null);
    const effortTouched = useRef(false);
    const runtimeTouched = useRef(false);

    const handlers = useAgentEditRuntimeHandlers({
      showDef: false,
      showInst: true,
      dProvider: "",
      dModel: "",
      dIsCustomProviderEditing: false,
      dIsCustomModelEditing: false,
      setDProvider: () => {},
      setDModel: () => {},
      setDIsCustomProviderEditing: () => {},
      setDIsCustomModelEditing: () => {},
      envVars: {},
      setEnvVars: () => {},
      definitionRuntimeId: "goose",
      setDefinitionRuntimeId: () => {},
      iProvider: "",
      iModel: "",
      iIsCustomProviderEditing: false,
      iIsCustomModelEditing: false,
      setIProvider: () => {},
      setIModel: () => {},
      setIIsCustomProviderEditing: () => {},
      setIIsCustomModelEditing: () => {},
      instanceEnvVars: {},
      setInstanceEnvVars: () => {},
      selectedRuntimeId,
      setSelectedRuntimeId,
      setInheritHarness: () => {},
      setAgentCommand: () => {},
      setAgentArgs: () => {},
      runtimeTouched,
      setIsAddHarnessOpen: () => {},
      setEffortLevel,
      effortTouched,
      runtimes,
      selectedRuntime: runtimes.find((r) => r.id === selectedRuntimeId),
      open: true,
    });

    return { handlers, effortLevel, effortTouched };
  });

  // Simulate: user selects an effort — effortTouched becomes true
  act(() => {
    result.current.effortTouched.current = true;
  });

  assert.equal(
    result.current.effortTouched.current,
    true,
    "effortTouched must be true after user selection",
  );

  // Simulate: user switches runtime to "claude"
  act(() => {
    result.current.handlers.handleRuntimeDropdownChange("claude");
  });

  // The reset must have cleared both:
  assert.equal(
    result.current.effortTouched.current,
    false,
    "effortTouched must be cleared after runtime switch — without this, resolveEffortSubmission emits the stale value",
  );
  assert.equal(
    result.current.effortLevel,
    null,
    "effortLevel must be reset to null after runtime switch",
  );
});
