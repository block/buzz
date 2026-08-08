/**
 * Behavior tests for the inline "Add custom harness…" dropdown entry shared by
 * AgentDefinitionDialog and AgentInstanceEditDialog.
 *
 * Two seams carry the feature, and both are pinned here:
 *
 *   1. ROUTING (`runtimeDropdownAction`) — the sentinel must resolve to "open
 *      the form", never to a selection. If it ever resolved to a selection the
 *      dialogs would write "\u0000add-custom-harness" into `runtime` and try to
 *      spawn an agent on a harness that does not exist.
 *   2. DEFERRED SELECTION (`usePendingHarnessSelection`) — saving only writes
 *      the definition file; the harness becomes a catalog entry when the
 *      invalidated discovery query refetches. Selecting on save would pick an
 *      id no entry backs. The hook must wait for the catalog, fire exactly
 *      once, stay silent when the user cancels, and drop the pending id when
 *      its dialog closes — the host dialogs stay mounted, so a stale id would
 *      otherwise select into reset form state on a later publish.
 *
 * The hook is mounted for real (react-dom/client + act) rather than simulated,
 * so its effect wiring — including the guard that survives the dialogs'
 * non-memoized change handlers — is what gets tested.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Minimal DOM shim ─────────────────────────────────────────────────────────
// react-dom/client needs a container element and a document; node has neither.
// The harness renders null, so no real node operations are exercised.

class ElementShim {
  constructor() {
    this.children = [];
    this.childNodes = [];
    this.nodeType = 1;
    this.nodeName = "DIV";
    this.tagName = "DIV";
    this.namespaceURI = "http://www.w3.org/1999/xhtml";
  }
  get ownerDocument() {
    return globalThis.document;
  }
  addEventListener() {}
  removeEventListener() {}
  appendChild(child) {
    this.children.push(child);
    this.childNodes.push(child);
    return child;
  }
  removeChild(child) {
    this.children = this.children.filter((current) => current !== child);
    this.childNodes = this.childNodes.filter((current) => current !== child);
    return child;
  }
  insertBefore(child) {
    return this.appendChild(child);
  }
  contains(target) {
    return this === target;
  }
}

globalThis.document = {
  activeElement: null,
  addEventListener() {},
  createElement: () => new ElementShim(),
  get defaultView() {
    return globalThis.window;
  },
  nodeType: 9,
  removeEventListener() {},
};
// react-dom derives update priority from window.event and walks iframe
// boundaries via window.HTMLIFrameElement during commit.
Object.defineProperty(globalThis, "window", {
  configurable: true,
  value: {
    addEventListener() {},
    document: globalThis.document,
    event: undefined,
    HTMLIFrameElement: ElementShim,
    removeEventListener() {},
  },
});
globalThis.HTMLElement = ElementShim;
globalThis.Node = ElementShim;
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";

import { NO_RUNTIME_DROPDOWN_VALUE } from "./agentConfigOptions.tsx";
import {
  ADD_CUSTOM_HARNESS_OPTION,
  ADD_CUSTOM_HARNESS_VALUE,
  readyHarnessId,
  runtimeDropdownAction,
  usePendingHarnessSelection,
} from "./addCustomHarness.ts";

// ── Routing: the sentinel opens the form, it is never a selection ────────────

test("selecting the add-custom entry requests the form and yields no runtime id", () => {
  const action = runtimeDropdownAction(ADD_CUSTOM_HARNESS_VALUE);
  assert.equal(action.kind, "add-custom-harness");
  // The dialogs read `action.runtimeId` on the select branch; the sentinel
  // must not carry one, or it could leak into form state.
  assert.equal("runtimeId" in action, false);
});

test("selecting a harness yields that harness id", () => {
  assert.deepEqual(runtimeDropdownAction("my-harness"), {
    kind: "select",
    runtimeId: "my-harness",
  });
});

test("selecting the no-runtime entry yields the empty id", () => {
  assert.deepEqual(runtimeDropdownAction(NO_RUNTIME_DROPDOWN_VALUE), {
    kind: "select",
    runtimeId: "",
  });
});

test("the add-custom sentinel cannot collide with a backend-valid harness id", () => {
  // Backend ids match [a-z0-9_][a-z0-9_-]* (custom_harnesses.rs), so a
  // NUL-prefixed value is unreachable as a real id.
  assert.equal(ADD_CUSTOM_HARNESS_VALUE.startsWith("\u0000"), true);
  assert.equal(ADD_CUSTOM_HARNESS_OPTION.value, ADD_CUSTOM_HARNESS_VALUE);
  assert.equal(ADD_CUSTOM_HARNESS_OPTION.label, "Add custom harness…");
});

// ── Readiness: an id is selectable only once the catalog publishes it ────────

test("a pending id absent from the catalog is not ready", () => {
  assert.equal(readyHarnessId([{ id: "claude" }], "my-harness"), null);
});

test("a pending id present in the catalog is ready", () => {
  assert.equal(
    readyHarnessId([{ id: "claude" }, { id: "my-harness" }], "my-harness"),
    "my-harness",
  );
});

test("no pending id is never ready even against a populated catalog", () => {
  assert.equal(readyHarnessId([{ id: "claude" }], null), null);
});

// ── Deferred selection: mounted hook ─────────────────────────────────────────

/**
 * Mount the real hook over a mutable catalog. Returns the setter the dialogs
 * call on save, a `setRuntimes` to simulate the discovery refetch, a `setOpen`
 * to simulate the owning dialog closing and reopening, and the log of ids the
 * hook handed back for selection.
 */
async function mountPendingSelection(initialRuntimes = []) {
  const selected = [];
  const control = {};

  function Harness() {
    const [runtimes, setRuntimes] = React.useState(initialRuntimes);
    const [open, setOpen] = React.useState(true);
    // Deliberately NOT memoized: both dialogs pass a plain function
    // declaration, so `onReady` has a fresh identity on every render.
    const onReady = (id) => selected.push(id);
    control.save = usePendingHarnessSelection(runtimes, onReady, open);
    control.setRuntimes = setRuntimes;
    control.setOpen = setOpen;
    return null;
  }

  const root = createRoot(new ElementShim());
  await act(async () => {
    root.render(React.createElement(Harness));
  });
  return { control, root, selected };
}

test("saving a harness selects it only once the catalog publishes it", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  // Save returns before discovery refetches — nothing to select yet.
  await act(async () => control.save("my-harness"));
  assert.deepEqual(selected, []);

  // The invalidated discovery query resolves with the new entry.
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "my-harness" }]),
  );
  assert.deepEqual(selected, ["my-harness"]);

  await act(async () => root.unmount());
});

test("a published harness is selected exactly once across later catalog updates", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  await act(async () => control.save("my-harness"));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "my-harness" }]),
  );
  assert.deepEqual(selected, ["my-harness"]);

  // Any later refetch re-renders with a new array identity and a new onReady
  // identity. Re-firing here would clobber a selection the user made in
  // between, so the pending id must have been cleared.
  await act(async () =>
    control.setRuntimes([
      { id: "claude" },
      { id: "my-harness" },
      { id: "codex" },
    ]),
  );
  assert.deepEqual(selected, ["my-harness"]);

  await act(async () => root.unmount());
});

test("cancelling the form leaves the current selection untouched", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  // Cancel never reports a saved id, so no selection is ever requested — even
  // as the catalog keeps refreshing underneath.
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "codex" }]),
  );
  assert.deepEqual(selected, []);

  await act(async () => root.unmount());
});

test("a saved harness discovery never publishes is never selected", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  // e.g. the definition file was written but the entry failed to load. The
  // hook must stall rather than select an id no catalog entry backs.
  await act(async () => control.save("ghost-harness"));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "codex" }]),
  );
  assert.deepEqual(selected, []);

  await act(async () => root.unmount());
});

test("two harnesses registered in a row are each selected when published", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  await act(async () => control.save("first"));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "first" }]),
  );
  await act(async () => control.save("second"));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "first" }, { id: "second" }]),
  );
  assert.deepEqual(selected, ["first", "second"]);

  await act(async () => root.unmount());
});

test("a second save before the first publishes selects only the later harness", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  // The dropdown holds one harness, so the latest registration wins: the
  // first id is dropped rather than queued behind the second.
  await act(async () => control.save("first"));
  await act(async () => control.save("second"));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "first" }, { id: "second" }]),
  );
  assert.deepEqual(selected, ["second"]);

  await act(async () => root.unmount());
});

// ── Lifecycle: a pending id never outlives the dialog that created it ────────

test("a harness published after its dialog closed is never selected", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  // Both host dialogs stay mounted when closed, so the hook keeps running.
  await act(async () => control.save("my-harness"));
  await act(async () => control.setOpen(false));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "my-harness" }]),
  );

  // Selecting here would write into form state the close already reset.
  assert.deepEqual(selected, []);

  await act(async () => root.unmount());
});

test("reopening after closing mid-registration does not select the abandoned harness", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  await act(async () => control.save("my-harness"));
  await act(async () => control.setOpen(false));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "my-harness" }]),
  );
  // The reopened dialog seeds from its own initial values; a stale pending id
  // must not overwrite them.
  await act(async () => control.setOpen(true));
  assert.deepEqual(selected, []);

  await act(async () => root.unmount());
});

test("a harness saved after reopening is still selected when published", async () => {
  const { control, root, selected } = await mountPendingSelection([
    { id: "claude" },
  ]);

  await act(async () => control.setOpen(false));
  await act(async () => control.setOpen(true));
  await act(async () => control.save("my-harness"));
  await act(async () =>
    control.setRuntimes([{ id: "claude" }, { id: "my-harness" }]),
  );
  assert.deepEqual(selected, ["my-harness"]);

  await act(async () => root.unmount());
});
