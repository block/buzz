/**
 * Behavior-level tests for the observer feed scroll-id wiring.
 *
 * Corrective actions addressed:
 *  1. Production derivation chain (via `deriveTranscriptBlockIds` — the same
 *     exported helper AgentSessionThreadPanel calls) + reference stability
 *     via useStableArrayShallow.
 *  4. Mode-toggle reset-key disjointness.
 *
 * Corrective action 3 (ordered DOM parity) — NAMED RESIDUAL:
 *  AgentSessionTranscriptList cannot render under node:test — the component
 *  tree transitively imports a .css file (BuzzLogoAnimation.tsx →
 *  buzz-logo-animation.css) and the test-loader has no CSS stub. The
 *  underlying invariant (outer-derived ids = inner data-message-id) is
 *  structurally guaranteed by both sides calling the same exported
 *  getDisplayBlockKey, but a full-component render test would additionally
 *  catch a block being filtered or the attribute being dropped. That
 *  coverage requires a CSS-stub addition to the test-loader (outside this
 *  PR's scope). See D1 in Paul's verification at event 8dfadd5f.
 *
 * Hook-level zero-write assertions (corrective action 2) and mode-toggle
 * re-pin behavior (corrective action 4) live in
 * useAnchoredScroll.observerScrollIds.test.mjs alongside the existing hook tests.
 */

import assert from "node:assert/strict";
import test from "node:test";

import React from "react";

import {
  deriveTranscriptBlockIds,
  getDisplayBlockKey,
  buildTranscriptDisplayBlocks,
} from "@/features/agents/ui/agentSessionTranscriptGrouping.ts";
import { observerEventScrollId } from "@/features/agents/ui/agentSessionPanelLayout.ts";

// ─── Helpers ────────────────────────────────────────────────────────────────────

const BASE_TS = "2026-07-08T00:00:00.000Z";

/**
 * Build a minimal ObserverEvent that produces a tool-call TranscriptItem
 * through `buildTranscriptState`. Uses the `session/update` → `tool_call_update`
 * method path (the most common tool-producing event in production).
 */
function mkToolEvent(
  seq,
  { sessionId = "sess-1", turnId = "turn-1", ts } = {},
) {
  return {
    seq,
    timestamp: ts ?? `2026-07-08T00:00:${String(seq).padStart(2, "0")}.000Z`,
    kind: "acp_read",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId,
    turnId,
    payload: {
      method: "session/update",
      params: {
        sessionId,
        update: {
          sessionUpdate: "tool_call_update",
          toolCallId: `call-${seq}`,
          toolName: `tool-${seq}`,
          status: "completed",
          args: "{}",
          result: "ok",
        },
      },
    },
  };
}

/** Build a minimal ObserverEvent for raw-mode id derivation. */
function mkRawEvent(seq, ts = BASE_TS) {
  return { seq, timestamp: ts };
}

// ── Corrective action 1: production derivation chain + reference stability ───
//
// These tests derive ids through the REAL production helper
// `deriveTranscriptBlockIds(events)` — the exact function
// `AgentSessionThreadPanel` calls. No mirror code, no pre-built items.

test("production chain: same-turn event append produces value-equal block id sequence", () => {
  const events1 = [mkToolEvent(1)];
  const ids1 = deriveTranscriptBlockIds(events1);

  // A second event on the same turn — block key unchanged.
  const events2 = [...events1, mkToolEvent(2)];
  const ids2 = deriveTranscriptBlockIds(events2);

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

test("production chain: streaming 10 same-turn events produces stable id sequence", () => {
  const events5 = Array.from({ length: 5 }, (_, i) => mkToolEvent(i + 1));
  const ids5 = deriveTranscriptBlockIds(events5);

  const events10 = [
    ...events5,
    ...Array.from({ length: 5 }, (_, i) => mkToolEvent(i + 6)),
  ];
  const ids10 = deriveTranscriptBlockIds(events10);

  assert.deepEqual(
    ids5,
    ids10,
    "same-turn streaming must not change block ids",
  );
});

test("production chain: new session produces value-different id sequence", () => {
  const events1 = [
    mkToolEvent(1, { sessionId: "sess-1", ts: "2026-07-08T00:00:01.000Z" }),
  ];
  const ids1 = deriveTranscriptBlockIds(events1);

  const events2 = [
    ...events1,
    mkToolEvent(2, {
      sessionId: "sess-2",
      turnId: "turn-2",
      ts: "2026-07-08T00:00:02.000Z",
    }),
  ];
  const ids2 = deriveTranscriptBlockIds(events2);

  assert.ok(ids2.length > ids1.length, "new session must grow the id list");
  assert.notDeepEqual(ids1, ids2);
});

test("production chain: new turn in same session produces value-different id sequence", () => {
  const events1 = [mkToolEvent(1)];
  const ids1 = deriveTranscriptBlockIds(events1);

  const events2 = [...events1, mkToolEvent(2, { turnId: "turn-2" })];
  const ids2 = deriveTranscriptBlockIds(events2);

  assert.equal(ids2.length, ids1.length + 1, "new turn adds one block id");
  assert.ok(
    ids2.some((id) => id.startsWith("turn:turn-2")),
    "new turn block key must be present",
  );
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

test("stabilization chain: production helper → useStableArrayShallow → same {id}[] reference", async () => {
  // End-to-end: call the PRODUCTION deriveTranscriptBlockIds from raw events,
  // stabilize, map to {id}[] — the exact chain in AgentSessionThreadPanel.
  const messageArrays = [];
  function Harness({ events }) {
    const blockIds = React.useMemo(
      () => deriveTranscriptBlockIds(events),
      [events],
    );
    const stableIds = useStableArrayShallow(blockIds);
    const messages = React.useMemo(
      () => stableIds.map((id) => ({ id })),
      [stableIds],
    );
    messageArrays.push(messages);
    return null;
  }

  const root = createRoot(document.createElement("div"));

  // First render: one tool event → one turn block.
  const events1 = [mkToolEvent(1)];
  await act(async () => {
    root.render(React.createElement(Harness, { events: events1 }));
  });

  // Second render: same turn, new event — block ids unchanged.
  const events2 = [...events1, mkToolEvent(2)];
  await act(async () => {
    root.render(React.createElement(Harness, { events: events2 }));
  });

  assert.ok(messageArrays.length >= 2);
  assert.equal(
    messageArrays[0],
    messageArrays[1],
    "messages reference must be preserved when block ids are value-equal " +
      "(this is the load-bearing invariant that prevents per-raw-event scrollTo writes)",
  );

  // Third render: new turn — block ids change → new reference.
  const events3 = [...events2, mkToolEvent(3, { turnId: "turn-2" })];
  await act(async () => {
    root.render(React.createElement(Harness, { events: events3 }));
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

// ── Corrective action 3: ordered DOM parity — structural guarantee ──────────
//
// AgentSessionTranscriptList cannot render under node:test (CSS import blocker:
// BuzzLogoAnimation.tsx → buzz-logo-animation.css). The test below verifies the
// structural guarantee: both sides (outer derivation + inner render) call the
// SAME getDisplayBlockKey function, and the outer uses the SAME
// buildTranscriptDisplayBlocks output. This cannot catch a block being filtered
// or the attribute being dropped by the component, but it does catch key-function
// drift. Full-component coverage requires a CSS-stub in the test-loader.

test("structural parity: getDisplayBlockKey output is ordered and deterministic across reorder", () => {
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
  const partialIds = partialBlocks.map(getDisplayBlockKey);

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
  const fullIds = fullBlocks.map(getDisplayBlockKey);

  // Both sequences have the same key identities.
  assert.deepEqual(
    [...partialIds].sort(),
    [...fullIds].sort(),
    "key identities must be stable across session_resolved",
  );

  // Each sequence is internally unique (no duplicates).
  assert.equal(
    new Set(partialIds).size,
    partialIds.length,
    "partial ids must be unique",
  );
  assert.equal(
    new Set(fullIds).size,
    fullIds.length,
    "full ids must be unique",
  );

  // Verify determinism: deriving again produces the same ordered sequence.
  const fullIds2 =
    buildTranscriptDisplayBlocks(fullItems).map(getDisplayBlockKey);
  assert.deepEqual(
    fullIds,
    fullIds2,
    "block id derivation must be deterministic (same input → same ordered output)",
  );
});

// ── Corrective action 4: mode-toggle reset-key disjointness ─────────────────

test("mode toggle: raw and transcript ids are in disjoint namespaces", () => {
  const events = [
    mkRawEvent(1, "2026-07-08T00:00:01.000Z"),
    mkRawEvent(2, "2026-07-08T00:00:02.000Z"),
  ];
  const rawIds = new Set(events.map((e) => observerEventScrollId(e)));

  // Derive transcript ids through the production helper.
  const toolEvents = [
    mkToolEvent(1, { ts: "2026-07-08T00:00:01.000Z" }),
    mkToolEvent(2, {
      sessionId: "sess-2",
      turnId: "turn-2",
      ts: "2026-07-08T00:00:02.000Z",
    }),
  ];
  const blockIds = deriveTranscriptBlockIds(toolEvents);

  for (const blockId of blockIds) {
    assert.ok(
      !rawIds.has(blockId),
      `block id "${blockId}" must not collide with any raw id — ` +
        "carrying an anchor across a mode toggle must never produce a false hit",
    );
  }
});
