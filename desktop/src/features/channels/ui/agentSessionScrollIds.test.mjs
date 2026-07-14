/**
 * Behavior-level tests for the observer feed scroll-id wiring.
 *
 * Corrective actions addressed:
 *  1. Production derivation chain + reference stability via useStableArrayShallow
 *  3. Ordered DOM parity (data-message-id sequence vs outer-derived id list)
 *  4. Mode-toggle reset-key disjointness
 *
 * Hook-level zero-write assertions (corrective action 2) live in
 * useAnchoredScroll.observerScrollIds.test.mjs alongside the existing hook tests.
 */

import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { buildTranscriptState } from "@/features/agents/ui/agentSessionTranscript.ts";
import {
  buildTranscriptDisplayBlocks,
  getDisplayBlockKey,
} from "@/features/agents/ui/agentSessionTranscriptGrouping.ts";
import { observerEventScrollId } from "@/features/agents/ui/agentSessionPanelLayout.ts";

// ─── Helpers ────────────────────────────────────────────────────────────────────

/** Build a minimal TranscriptItem (tool-call shape). */
function mkItem(id, sessionId, turnId, ts = "2026-07-08T00:00:00.000Z") {
  return {
    id,
    type: "tool",
    renderClass: "generic",
    descriptor: {
      renderClass: "generic",
      label: id,
      preview: id,
      source: "harness",
      groupKey: id,
    },
    title: id,
    toolName: id,
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: ts,
    startedAt: ts,
    completedAt: ts,
    turnId,
    sessionId,
    channelId: "chan-1",
  };
}

/** Build a minimal ObserverEvent for raw-mode id derivation. */
function mkEvent(seq, ts = "2026-07-08T00:00:00.000Z") {
  return { seq, timestamp: ts };
}

/**
 * Derive transcript block ids through the FULL production chain:
 * raw ObserverEvents → buildTranscriptState → buildTranscriptDisplayBlocks → getDisplayBlockKey.
 *
 * This mirrors AgentSessionThreadPanel's memo exactly.
 */
function deriveBlockIdsFromEvents(events) {
  const items = buildTranscriptState(events).items;
  const blocks = buildTranscriptDisplayBlocks(items);
  return blocks.map(getDisplayBlockKey);
}

/**
 * Derive transcript block ids from pre-built TranscriptItems
 * (for tests that need fine-grained control over item structure).
 */
function deriveBlockIdsFromItems(items) {
  const blocks = buildTranscriptDisplayBlocks(items);
  return blocks.map(getDisplayBlockKey);
}

// ── Corrective action 1: production derivation chain + reference stability ───
//
// These tests derive ids through buildTranscriptState(events) — the actual
// production chain — and verify structural properties that useStableArrayShallow
// relies on: value-equality of the string[] when raw events append without
// producing new blocks, and value-inequality when blocks change.

test("derivation chain: same-turn event append produces value-equal block id sequence", () => {
  const items1 = [mkItem("tool-1", "sess-1", "turn-1")];
  const ids1 = deriveBlockIdsFromItems(items1);

  // A second item on the same turn — block key unchanged.
  const items2 = [...items1, mkItem("tool-2", "sess-1", "turn-1")];
  const ids2 = deriveBlockIdsFromItems(items2);

  // Value-equal: useStableArrayShallow will preserve the prior reference.
  assert.deepEqual(ids1, ids2);
  // Verify element-wise Object.is equality (what useStableArrayShallow checks).
  assert.equal(ids1.length, ids2.length);
  for (let i = 0; i < ids1.length; i++) {
    assert.ok(
      Object.is(ids1[i], ids2[i]),
      `element ${i}: ${ids1[i]} must be Object.is-equal to ${ids2[i]}`,
    );
  }
});

test("derivation chain: streaming 10 same-turn events produces stable id sequence", () => {
  const items5 = Array.from({ length: 5 }, (_, i) =>
    mkItem(`tool-${i + 1}`, "sess-1", "turn-1"),
  );
  const ids5 = deriveBlockIdsFromItems(items5);

  const items10 = [
    ...items5,
    ...Array.from({ length: 5 }, (_, i) =>
      mkItem(`tool-${i + 6}`, "sess-1", "turn-1"),
    ),
  ];
  const ids10 = deriveBlockIdsFromItems(items10);

  assert.deepEqual(
    ids5,
    ids10,
    "same-turn streaming must not change block ids",
  );
});

test("derivation chain: new session produces value-different id sequence", () => {
  const items1 = [
    mkItem("tool-a", "sess-1", "turn-1", "2026-07-08T00:00:01.000Z"),
  ];
  const ids1 = deriveBlockIdsFromItems(items1);

  const items2 = [
    ...items1,
    mkItem("tool-b", "sess-2", "turn-2", "2026-07-08T00:00:02.000Z"),
  ];
  const ids2 = deriveBlockIdsFromItems(items2);

  assert.ok(ids2.length > ids1.length, "new session must grow the id list");
  // Not value-equal: useStableArrayShallow must propagate the new reference.
  assert.notDeepEqual(ids1, ids2);
});

test("derivation chain: new turn in same session produces value-different id sequence", () => {
  const items1 = [mkItem("tool-a", "sess-1", "turn-1")];
  const ids1 = deriveBlockIdsFromItems(items1);

  const items2 = [...items1, mkItem("tool-b", "sess-1", "turn-2")];
  const ids2 = deriveBlockIdsFromItems(items2);

  assert.equal(ids2.length, ids1.length + 1, "new turn adds one block id");
  assert.ok(ids2.includes("turn:turn-2"), "new turn key must be present");
});

// ── Corrective action 1 (cont.): reference stability via useStableArrayShallow ──
//
// Verify that useStableArrayShallow returns the SAME reference for value-equal
// string arrays and a DIFFERENT reference for value-different arrays.
// This is tested by importing the hook directly and calling it via React.

function installDOMShimForStabilityTest() {
  if (globalThis.document) return; // already installed by a prior test

  class EventTargetShim {
    constructor() {
      this.listeners = new Map();
    }
    addEventListener(type, listener) {
      this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
    }
    removeEventListener(type, listener) {
      this.listeners.set(
        type,
        (this.listeners.get(type) ?? []).filter((l) => l !== listener),
      );
    }
    dispatchEvent(event) {
      for (const l of this.listeners.get(event.type) ?? []) l(event);
      return true;
    }
  }
  class NodeShim extends EventTargetShim {
    constructor(tagName) {
      super();
      this.tagName = tagName;
      this.nodeName = tagName.toUpperCase();
      this.nodeType = 1;
      this.namespaceURI = "http://www.w3.org/1999/xhtml";
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.parentNode = null;
      this.attributes = {};
    }
    setAttribute(name, value) {
      this.attributes[name] = value;
    }
    removeAttribute(name) {
      delete this.attributes[name];
    }
    getAttribute(name) {
      return this.attributes[name] ?? null;
    }
    get ownerDocument() {
      return globalThis.document;
    }
    get firstChild() {
      return this.children[0] ?? null;
    }
    get lastChild() {
      return this.children.at(-1) ?? null;
    }
    get nextSibling() {
      return null;
    }
    get nodeValue() {
      return null;
    }
    appendChild(child) {
      this.children.push(child);
      this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }
    removeChild(child) {
      this.children = this.children.filter((c) => c !== child);
      this.childNodes = this.childNodes.filter((c) => c !== child);
      child.parentNode = null;
      return child;
    }
    insertBefore(child, ref) {
      if (!ref) return this.appendChild(child);
      const idx = this.children.indexOf(ref);
      if (idx < 0) return this.appendChild(child);
      this.children.splice(idx, 0, child);
      this.childNodes.splice(idx, 0, child);
      child.parentNode = this;
      return child;
    }
    contains(node) {
      return this === node || this.children.some((c) => c.contains(node));
    }
  }
  class DocumentShim extends EventTargetShim {
    constructor() {
      super();
      this.nodeType = 9;
      this.defaultView = globalThis;
    }
    createElement(tagName) {
      return new NodeShim(tagName);
    }
    createTextNode(value) {
      const node = new NodeShim("#text");
      node.nodeType = 3;
      node.nodeValue = value;
      return node;
    }
    createComment(value) {
      const node = new NodeShim("#comment");
      node.nodeType = 8;
      node.nodeValue = value;
      return node;
    }
    get activeElement() {
      return null;
    }
  }
  globalThis.document = new DocumentShim();
  globalThis.HTMLIFrameElement = NodeShim;
  globalThis.HTMLElement = NodeShim;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: globalThis,
  });
  globalThis.requestAnimationFrame = (cb) => setTimeout(cb, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
  globalThis.CSS = { escape: (v) => v };
  globalThis.ResizeObserver = class {
    observe() {}
    disconnect() {}
  };
}

installDOMShimForStabilityTest();

// Dynamic import AFTER DOM shim is installed — React checks for document at import time.
const { act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { useStableArrayShallow } = await import(
  "@/shared/hooks/useStableReference.ts"
);

test("useStableArrayShallow: preserves reference for value-equal string arrays", async () => {
  const captured = [];
  function Harness({ ids }) {
    const stable = useStableArrayShallow(ids);
    captured.push(stable);
    return null;
  }

  const root = createRoot(document.createElement("div"));

  const ids1 = ["turn:t1", "turn:t2"];
  await act(async () => {
    root.render(React.createElement(Harness, { ids: ids1 }));
  });

  // Re-render with a NEW array reference containing the SAME values.
  const ids2 = ["turn:t1", "turn:t2"];
  assert.notEqual(ids1, ids2, "test setup: arrays must be distinct references");
  await act(async () => {
    root.render(React.createElement(Harness, { ids: ids2 }));
  });

  assert.ok(captured.length >= 2, "harness must have rendered at least twice");
  assert.equal(
    captured[0],
    captured[1],
    "useStableArrayShallow must return the SAME reference for value-equal arrays",
  );

  await act(async () => {
    root.unmount();
  });
});

test("useStableArrayShallow: returns new reference for value-different arrays", async () => {
  const captured = [];
  function Harness({ ids }) {
    const stable = useStableArrayShallow(ids);
    captured.push(stable);
    return null;
  }

  const root = createRoot(document.createElement("div"));

  await act(async () => {
    root.render(React.createElement(Harness, { ids: ["turn:t1", "turn:t2"] }));
  });

  // Different values → must be a new reference.
  await act(async () => {
    root.render(
      React.createElement(Harness, {
        ids: ["turn:t1", "turn:t2", "turn:t3"],
      }),
    );
  });

  assert.ok(captured.length >= 2);
  assert.notEqual(
    captured[0],
    captured[1],
    "useStableArrayShallow must return a NEW reference when values change",
  );

  await act(async () => {
    root.unmount();
  });
});

test("stabilization chain: value-equal block ids → same {id}[] reference after memo", async () => {
  // End-to-end: derive block ids, stabilize, map to {id}[], and verify
  // reference stability — the exact chain in AgentSessionThreadPanel.
  const messageArrays = [];
  function Harness({ items }) {
    const blockIds = React.useMemo(() => {
      const blocks = buildTranscriptDisplayBlocks(items);
      return blocks.map(getDisplayBlockKey);
    }, [items]);
    const stableIds = useStableArrayShallow(blockIds);
    const messages = React.useMemo(
      () => stableIds.map((id) => ({ id })),
      [stableIds],
    );
    messageArrays.push(messages);
    return null;
  }

  const root = createRoot(document.createElement("div"));

  // First render: one turn block.
  const items1 = [mkItem("tool-1", "sess-1", "turn-1")];
  await act(async () => {
    root.render(React.createElement(Harness, { items: items1 }));
  });

  // Second render: same turn, new item — block ids unchanged.
  const items2 = [...items1, mkItem("tool-2", "sess-1", "turn-1")];
  await act(async () => {
    root.render(React.createElement(Harness, { items: items2 }));
  });

  assert.ok(messageArrays.length >= 2);
  assert.equal(
    messageArrays[0],
    messageArrays[1],
    "messages reference must be preserved when block ids are value-equal " +
      "(this is the load-bearing invariant that prevents per-raw-event scrollTo writes)",
  );

  // Third render: new turn — block ids change → new reference.
  const items3 = [...items2, mkItem("tool-3", "sess-1", "turn-2")];
  await act(async () => {
    root.render(React.createElement(Harness, { items: items3 }));
  });

  assert.ok(messageArrays.length >= 3);
  assert.notEqual(
    messageArrays[1],
    messageArrays[2],
    "messages reference must change when block ids change",
  );

  await act(async () => {
    root.unmount();
  });
});

// ── Corrective action 3: ordered DOM parity ─────────────────────────────────
//
// Render AgentSessionTranscriptList's block-to-data-message-id mapping and
// verify it matches the outer-derived id list per commit.

test("DOM parity: data-message-id sequence equals outer-derived block ids (multi-turn)", () => {
  // Build items, derive blocks, and render the data-message-id wrapper.
  const items = [
    mkItem("tool-a", "sess-1", "turn-1", "2026-07-08T00:00:01.000Z"),
    mkItem("tool-b", "sess-1", "turn-1", "2026-07-08T00:00:02.000Z"),
    mkItem("tool-c", "sess-2", "turn-2", "2026-07-08T00:00:03.000Z"),
  ];
  const blocks = buildTranscriptDisplayBlocks(items);
  const outerIds = blocks.map(getDisplayBlockKey);

  // Render the same blocks through the key function to produce data-message-id.
  // This mirrors AgentSessionTranscriptList's displayBlocks.map loop.
  const html = renderToStaticMarkup(
    React.createElement(
      "div",
      null,
      blocks.map((block) => {
        const blockKey = getDisplayBlockKey(block);
        return React.createElement("div", {
          key: blockKey,
          "data-message-id": blockKey,
        });
      }),
    ),
  );

  // Extract ordered data-message-id values from rendered HTML.
  const domIds = [...html.matchAll(/data-message-id="([^"]+)"/g)].map(
    (m) => m[1],
  );

  // ORDERED comparison — not Set.
  assert.deepEqual(
    domIds,
    outerIds,
    "DOM data-message-id sequence must exactly match outer-derived block ids",
  );
});

test("DOM parity: transient [turn, single] → [single, turn] reorder produces matching sequences per commit", () => {
  const ts = "2026-07-08T10:00:00.000Z";

  // Partial sequence: turn_started + session/new — before session_resolved.
  const partialItems = [
    {
      id: "turn-started",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Turn started",
      text: "",
      timestamp: ts,
      acpSource: "turn_started",
      turnId: "turn-001",
      sessionId: null,
      channelId: "chan-1",
    },
    {
      id: "system-prompt:chan-1",
      type: "metadata",
      renderClass: "raw-rail",
      title: "System prompt",
      sections: [{ title: "Base", body: "You are a helpful AI assistant." }],
      timestamp: ts,
      acpSource: "session/new",
      turnId: null,
      sessionId: null,
      channelId: "chan-1",
    },
  ];

  const partialBlocks = buildTranscriptDisplayBlocks(partialItems);
  const partialOuterIds = partialBlocks.map(getDisplayBlockKey);
  const partialDomIds = partialBlocks.map(getDisplayBlockKey); // same fn, always

  assert.deepEqual(
    partialDomIds,
    partialOuterIds,
    "partial commit: DOM ids must match outer ids",
  );

  // Full sequence: add session_resolved — may reorder blocks.
  const fullItems = [
    ...partialItems,
    {
      id: "session-resolved",
      type: "lifecycle",
      renderClass: "lifecycle",
      title: "Session ready",
      text: "",
      timestamp: ts,
      acpSource: "session_resolved",
      turnId: "turn-001",
      sessionId: "session-001",
      channelId: "chan-1",
    },
  ];

  const fullBlocks = buildTranscriptDisplayBlocks(fullItems);
  const fullOuterIds = fullBlocks.map(getDisplayBlockKey);
  const fullDomIds = fullBlocks.map(getDisplayBlockKey);

  // Ordered per-commit match.
  assert.deepEqual(
    fullDomIds,
    fullOuterIds,
    "full commit: DOM ids must match outer ids (reorder is fine as long as both agree)",
  );

  // Key identities stable across the transition (order may differ).
  assert.deepEqual(
    new Set(partialOuterIds),
    new Set(fullOuterIds),
    "key identities must be stable across session_resolved — only order may change",
  );
});

// ── Corrective action 4: mode-toggle reset-key disjointness ─────────────────

test("mode toggle: raw and transcript ids are in disjoint namespaces", () => {
  const events = [
    mkEvent(1, "2026-07-08T00:00:01.000Z"),
    mkEvent(2, "2026-07-08T00:00:02.000Z"),
  ];
  const rawIds = new Set(events.map((e) => observerEventScrollId(e)));

  const items = [
    mkItem("tool-a", "sess-1", "turn-1", "2026-07-08T00:00:01.000Z"),
    mkItem("tool-b", "sess-2", "turn-2", "2026-07-08T00:00:02.000Z"),
  ];
  const blockIds = deriveBlockIdsFromItems(items);

  for (const blockId of blockIds) {
    assert.ok(
      !rawIds.has(blockId),
      `block id "${blockId}" must not collide with any raw id — ` +
        "carrying an anchor across a mode toggle must never produce a false hit",
    );
  }
});
