import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

// useAgentEditRuntimeHandlers is a React hook (it calls
// usePendingHarnessSelection internally), so it must be mounted to exercise
// the real handler wiring. JSDOM provides the DOM globals renderHook needs.
const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
    localStorage: dom.window.localStorage,
  });
});

after(() => dom.window.close());

let hookModule;
before(async () => {
  hookModule = await import("./useAgentEditRuntimeHandlers.ts");
});

// Build a RuntimeHandlersInput whose every setter records its calls, so a test
// can assert which layer (definition vs instance) a handler wrote to.
function makeInput(overrides = {}) {
  const calls = {
    setDefinitionRuntimeId: [],
    setSelectedRuntimeId: [],
    setInheritHarness: [],
    setAgentCommand: [],
    setAgentArgs: [],
    setDProvider: [],
    setDModel: [],
    setIProvider: [],
    setIModel: [],
    runtimeTouchedWrites: 0,
  };
  const noop = () => {};
  const runtimeTouched = {
    get current() {
      return this._v ?? false;
    },
    set current(v) {
      this._v = v;
      calls.runtimeTouchedWrites++;
    },
  };
  const input = {
    showDef: true,
    showInst: true,
    dProvider: "openai",
    dModel: "gpt-4o",
    dIsCustomProviderEditing: false,
    dIsCustomModelEditing: false,
    setDProvider: (v) => calls.setDProvider.push(v),
    setDModel: (v) => calls.setDModel.push(v),
    setDIsCustomProviderEditing: noop,
    setDIsCustomModelEditing: noop,
    envVars: {},
    setEnvVars: noop,
    definitionRuntimeId: "goose",
    setDefinitionRuntimeId: (v) => calls.setDefinitionRuntimeId.push(v),
    iProvider: "openai",
    iModel: "gpt-4o",
    iIsCustomProviderEditing: false,
    iIsCustomModelEditing: false,
    setIProvider: (v) => calls.setIProvider.push(v),
    setIModel: (v) => calls.setIModel.push(v),
    setIIsCustomProviderEditing: noop,
    setIIsCustomModelEditing: noop,
    instanceEnvVars: {},
    setInstanceEnvVars: noop,
    selectedRuntimeId: "goose",
    setSelectedRuntimeId: (v) => calls.setSelectedRuntimeId.push(v),
    setInheritHarness: (v) => calls.setInheritHarness.push(v),
    setAgentCommand: (v) => calls.setAgentCommand.push(v),
    setAgentArgs: (v) => calls.setAgentArgs.push(v),
    runtimeTouched,
    setIsAddHarnessOpen: noop,
    setEffortLevel: noop,
    effortTouched: { current: false },
    runtimes: [{ id: "buzz-agent", command: "", defaultArgs: [] }],
    selectedRuntime: { id: "goose", command: "", defaultArgs: [] },
    open: true,
    ...overrides,
  };
  return { input, calls };
}

async function renderHandlers(input) {
  const { renderHook } = await import("@testing-library/react");
  return renderHook(() => hookModule.useAgentEditRuntimeHandlers(input));
}

// ── P1: Relay Mesh selection must switch the runtime on the SHOWN layer ───────
//
// Carl's finding: `handleProviderDropdownChange` forced buzz-agent through the
// INSTANCE handler unconditionally. In a linked edit (showDef + showInst) that
// pinned the clicked instance to buzz-agent while the definition kept its old
// runtime — a D/I ownership-boundary violation. Selecting Relay Mesh while a
// definition is shown must change the DEFINITION runtime and leave the instance
// pin untouched. Reverting the fix (routing back through the instance handler,
// or consulting the instance runtime id) makes both assertions below fail.

test("test_relay_mesh_in_linked_edit_switches_definition_runtime_not_instance", async () => {
  const { act } = await import("@testing-library/react");
  const { input, calls } = makeInput();
  const { result } = await renderHandlers(input);

  act(() => result.current.handleProviderDropdownChange("relay-mesh"));

  assert.deepEqual(
    calls.setDefinitionRuntimeId,
    ["buzz-agent"],
    "definition runtime must switch to buzz-agent when Relay Mesh is chosen with the definition shown",
  );
  assert.deepEqual(
    calls.setSelectedRuntimeId,
    [],
    "the instance runtime pin must NOT be touched in a linked edit",
  );
  assert.equal(
    calls.runtimeTouchedWrites,
    0,
    "the instance runtimeTouched ref must NOT be set in a linked edit",
  );
  assert.deepEqual(
    calls.setInheritHarness,
    [],
    "instance harness inheritance must NOT be cleared in a linked edit",
  );
  // The definition provider/model still land on the relay-mesh selection.
  assert.equal(
    calls.setDProvider.at(-1),
    "relay-mesh",
    "definition provider must become relay-mesh",
  );
  assert.equal(
    calls.setDModel.at(-1),
    "auto",
    "definition model must become auto for relay-mesh",
  );
});

test("test_relay_mesh_in_instance_only_edit_switches_instance_runtime", async () => {
  const { act } = await import("@testing-library/react");
  // Instance-only context: no definition shown, so the instance handler is the
  // correct layer and buzz-agent must be pinned on the instance.
  const { input, calls } = makeInput({
    showDef: false,
    showInst: true,
    selectedRuntimeId: "goose",
    definitionRuntimeId: "goose",
  });
  const { result } = await renderHandlers(input);

  act(() => result.current.handleProviderDropdownChange("relay-mesh"));

  assert.deepEqual(
    calls.setSelectedRuntimeId,
    ["buzz-agent"],
    "instance runtime must switch to buzz-agent in an instance-only edit",
  );
  assert.deepEqual(
    calls.setDefinitionRuntimeId,
    [],
    "definition runtime must NOT be touched in an instance-only edit",
  );
  assert.equal(
    calls.setIProvider.at(-1),
    "relay-mesh",
    "instance provider must become relay-mesh",
  );
});
