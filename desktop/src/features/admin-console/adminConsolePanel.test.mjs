/**
 * Behavior and race tests for AdminConsoleSettingsCard / AdminConsoleSettingsSession.
 *
 * Tests mount the REAL production components (including the key-prop session
 * boundary, sessionTokenRef fence, and abortAndResetProbe wiring) against a
 * mocked Tauri IPC bridge and a real QueryClientProvider.
 *
 * This file uses the hand-rolled MinimalDocument shim (same pattern as
 * useLoadArchivedObserverEvents.test.mjs) and covers prop-driven and query-
 * driven tests that do NOT require native event dispatch through React 19's
 * container-level delegation:
 *
 * What makes these tests authoritative — they fail if:
 *   - `pubkeyHex ? <Session …> : null` render gate removed (authorized-logout-teardown)
 *   - `key={pubkeyHex}` boundary is removed (identity-switch test)
 *   - `active` flag cleanup is removed from useAsyncLoad (old-list-after-new-list)
 *   - the `getAdminOrigin()` catch is changed to silent-degrade (storage-error test)
 *
 * authorized-logout-teardown lives here (MinimalDocument, not jsdom) because the test is
 * query-driven (act + qc.setQueryData + settle), not event-driven. The MinimalDocument
 * suite handles async transitions cleanly without the jsdom global scheduler.
 *
 * Cross-identity delayed-save and all event-driven tests (origin-edit, detail-navigation,
 * blob-leak-on-back-navigation, same-session-save-race) live in adminConsolePanelSession.jsdom-test.mjs
 * where fireEvent dispatches native events through React 19's container-level delegation.
 *
 * Also covers:
 *   - parseImetaAttachments wire contract (imported from AdminConsolePanel)
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

// ── Minimal DOM shim ──────────────────────────────────────────────────────────
//
// Installs the minimum DOM surface that React + react-dom/client need.
// Uses the same pattern as useLoadArchivedObserverEvents.test.mjs to avoid
// jsdom background timers that prevent the process from exiting cleanly.

function installDOMShim() {
  class MinimalEventTarget {
    constructor() {
      this._listeners = {};
    }
    addEventListener(type, fn) {
      if (!this._listeners[type]) this._listeners[type] = [];
      this._listeners[type].push(fn);
    }
    removeEventListener(type, fn) {
      if (this._listeners[type]) {
        this._listeners[type] = this._listeners[type].filter((f) => f !== fn);
      }
    }
    dispatchEvent(e) {
      for (const fn of this._listeners[e.type] ?? []) fn(e);
      return true;
    }
  }

  class MinimalNode extends MinimalEventTarget {
    constructor(tagName) {
      super();
      this.tagName = tagName?.toUpperCase?.() ?? tagName;
      this.nodeName = this.tagName;
      this.children = [];
      this.childNodes = [];
      this.style = {};
      this.nodeType = 1;
      this.parentNode = null;
      this.attributes = [];
      this._data = {};
    }
    get ownerDocument() {
      return globalThis.document;
    }
    get firstChild() {
      return this.childNodes[0] ?? null;
    }
    get lastChild() {
      return this.childNodes[this.childNodes.length - 1] ?? null;
    }
    get nextSibling() {
      return null;
    }
    get previousSibling() {
      return null;
    }
    get nodeValue() {
      return null;
    }
    set nodeValue(_v) {}
    get textContent() {
      return this.childNodes.map((c) => c.textContent ?? "").join("");
    }
    set textContent(v) {
      this.childNodes = [];
      if (v) {
        const t = globalThis.document.createTextNode(v);
        this.appendChild(t);
      }
    }
    appendChild(child) {
      child.parentNode = this;
      this.childNodes.push(child);
      if (child.nodeType === 1) this.children.push(child);
      return child;
    }
    removeChild(child) {
      this.childNodes = this.childNodes.filter((c) => c !== child);
      this.children = this.children.filter((c) => c !== child);
      return child;
    }
    insertBefore(newNode, refNode) {
      if (!refNode) return this.appendChild(newNode);
      const i = this.childNodes.indexOf(refNode);
      if (i < 0) return this.appendChild(newNode);
      newNode.parentNode = this;
      this.childNodes.splice(i, 0, newNode);
      if (newNode.nodeType === 1) this.children.push(newNode);
      return newNode;
    }
    replaceChild(newNode, oldNode) {
      const i = this.childNodes.indexOf(oldNode);
      if (i >= 0) {
        newNode.parentNode = this;
        this.childNodes[i] = newNode;
        const j = this.children.indexOf(oldNode);
        if (j >= 0) this.children[j] = newNode;
      }
      return oldNode;
    }
    contains(node) {
      if (!node) return false;
      return this === node || this.childNodes.some((c) => c?.contains?.(node));
    }
    setAttribute(name, value) {
      this._data[name] = value;
    }
    getAttribute(name) {
      return this._data[name] ?? null;
    }
    hasAttribute(name) {
      return Object.hasOwn(this._data, name);
    }
    removeAttribute(name) {
      delete this._data[name];
    }
    querySelector(selector) {
      // Support [data-testid='...'] and simple tag selectors.
      const attrMatch = selector.match(/\[([^\]=']+)(?:='([^']*)')?\]/);
      const tagMatch = selector.match(/^([a-zA-Z]+)$/);
      for (const node of this._allElements()) {
        if (attrMatch) {
          const [, attrName, attrVal] = attrMatch;
          const nodeVal = node.getAttribute?.(attrName);
          if (attrVal === undefined ? nodeVal !== null : nodeVal === attrVal) {
            return node;
          }
        } else if (tagMatch) {
          if (node.tagName?.toLowerCase() === tagMatch[1].toLowerCase()) {
            return node;
          }
        }
      }
      return null;
    }
    querySelectorAll(selector) {
      const attrMatch = selector.match(/\[([^\]=']+)(?:='([^']*)')?\]/);
      const results = [];
      for (const node of this._allElements()) {
        if (attrMatch) {
          const [, attrName, attrVal] = attrMatch;
          const nodeVal = node.getAttribute?.(attrName);
          if (attrVal === undefined ? nodeVal !== null : nodeVal === attrVal) {
            results.push(node);
          }
        }
      }
      return results;
    }
    *_allElements() {
      for (const child of this.childNodes) {
        yield child;
        if (child._allElements) yield* child._allElements();
      }
    }
    get innerHTML() {
      return this.childNodes
        .map((c) => c.outerHTML ?? c.textContent ?? "")
        .join("");
    }
    set innerHTML(_v) {}
    get outerHTML() {
      return `<${this.tagName?.toLowerCase() ?? "div"}>...</${this.tagName?.toLowerCase() ?? "div"}>`;
    }
    focus() {}
    blur() {}
    getBoundingClientRect() {
      return { top: 0, left: 0, bottom: 0, right: 0, width: 0, height: 0 };
    }
    cloneNode() {
      return new MinimalNode(this.tagName);
    }
    get value() {
      return this._value ?? "";
    }
    set value(v) {
      this._value = v;
    }
    get disabled() {
      return this._disabled ?? false;
    }
    set disabled(v) {
      this._disabled = v;
    }
    get type() {
      return this._type ?? "";
    }
    set type(v) {
      this._type = v;
    }
    get checked() {
      return this._checked ?? false;
    }
    set checked(v) {
      this._checked = v;
    }
    get className() {
      return this._className ?? "";
    }
    set className(v) {
      this._className = v;
    }
    get id() {
      return this._id ?? "";
    }
    set id(v) {
      this._id = v;
    }
    get placeholder() {
      return this._placeholder ?? "";
    }
    set placeholder(v) {
      this._placeholder = v;
    }
    get readOnly() {
      return this._readOnly ?? false;
    }
    set readOnly(v) {
      this._readOnly = v;
    }
    get tabIndex() {
      return this._tabIndex ?? -1;
    }
    set tabIndex(v) {
      this._tabIndex = v;
    }
    get href() {
      return this._href ?? "";
    }
    set href(v) {
      this._href = v;
    }
    get src() {
      return this._src ?? "";
    }
    set src(v) {
      this._src = v;
    }
    get alt() {
      return this._alt ?? "";
    }
    set alt(v) {
      this._alt = v;
    }
  }

  class MinimalTextNode extends MinimalEventTarget {
    constructor(value) {
      super();
      this.nodeType = 3;
      this.nodeName = "#text";
      this.nodeValue = value;
      this.parentNode = null;
    }
    get textContent() {
      return this.nodeValue;
    }
    set textContent(v) {
      this.nodeValue = v;
    }
    contains(node) {
      return this === node;
    }
  }

  class MinimalDocument extends MinimalEventTarget {
    constructor() {
      super();
      this.nodeType = 9;
      this.nodeName = "#document";
      this._body = null;
      this._head = null;
    }
    createElement(tagName) {
      return new MinimalNode(tagName);
    }
    createTextNode(value) {
      return new MinimalTextNode(value);
    }
    createComment(value) {
      const n = new MinimalNode("#comment");
      n.nodeType = 8;
      n.nodeValue = value;
      return n;
    }
    createElementNS(_ns, tagName) {
      return this.createElement(tagName);
    }
    get body() {
      if (!this._body) {
        this._body = this.createElement("body");
      }
      return this._body;
    }
    get head() {
      if (!this._head) {
        this._head = this.createElement("head");
      }
      return this._head;
    }
    get activeElement() {
      return null;
    }
    contains(node) {
      return node != null;
    }
    querySelector(sel) {
      return this.body.querySelector(sel);
    }
    querySelectorAll(sel) {
      return this.body.querySelectorAll(sel);
    }
    get documentElement() {
      return this.body;
    }
  }

  const doc = new MinimalDocument();
  globalThis.document = doc;
  globalThis.HTMLElement = MinimalNode;
  globalThis.HTMLInputElement = MinimalNode;
  globalThis.HTMLButtonElement = MinimalNode;
  globalThis.HTMLDivElement = MinimalNode;
  globalThis.HTMLSpanElement = MinimalNode;
  globalThis.HTMLAnchorElement = MinimalNode;
  globalThis.HTMLFormElement = MinimalNode;
  globalThis.HTMLIFrameElement = MinimalNode;
  globalThis.SVGElement = MinimalNode;
  globalThis.SVGSVGElement = MinimalNode;
  globalThis.Text = MinimalTextNode;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";

  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);

  globalThis.MutationObserver = class {
    observe() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  };

  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };

  globalThis.IntersectionObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };

  globalThis.getComputedStyle = () => ({
    getPropertyValue: () => "",
    setProperty: () => {},
  });

  if (typeof globalThis.window === "undefined") {
    Object.defineProperty(globalThis, "window", {
      value: globalThis,
      configurable: true,
    });
  }
  if (!Object.getOwnPropertyDescriptor(globalThis, "navigator")?.value) {
    Object.defineProperty(globalThis, "navigator", {
      value: { userAgent: "node" },
      configurable: true,
    });
  }
  // CommunitiesProvider reads localStorage on mount; provide a minimal no-op
  // shim so tests that wrap components needing CommunitiesProvider don't throw.
  if (typeof globalThis.localStorage === "undefined") {
    const _store = Object.create(null);
    Object.defineProperty(globalThis, "localStorage", {
      value: {
        getItem: (k) => _store[k] ?? null,
        setItem: (k, v) => {
          _store[k] = String(v);
        },
        removeItem: (k) => {
          delete _store[k];
        },
        clear: () => {
          for (const k of Object.keys(_store)) delete _store[k];
        },
        get length() {
          return Object.keys(_store).length;
        },
        key: (i) => Object.keys(_store)[i] ?? null,
      },
      configurable: true,
    });
  }
}

installDOMShim();

// ── Tauri IPC interceptor ─────────────────────────────────────────────────────

/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
const ipcHandlers = new Map();

function setIpcHandler(cmd, fn) {
  ipcHandlers.set(cmd, fn);
}
function clearIpcHandlers() {
  ipcHandlers.clear();
}

globalThis.__TAURI_INTERNALS__ = {
  invoke(cmd, args) {
    const handler = ipcHandlers.get(cmd);
    if (handler) return handler(args);
    return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
  },
  transformCallback(_cb) {
    return Math.random();
  },
};

// ── Production imports ────────────────────────────────────────────────────────

import React from "react";
import { createRoot } from "react-dom/client";
import { act } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { CommunitiesProvider } from "@/features/communities/useCommunities.tsx";

import { AdminConsoleSettingsCard } from "./AdminConsoleSettingsCard.tsx";
import {
  AdminConsolePanel,
  parseImetaAttachments,
} from "./AdminConsolePanel.tsx";
import { applyAttachmentBudget } from "./AdminConsoleFeedbackTab.tsx";

// ── Deferred promise helper ───────────────────────────────────────────────────

function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// ── Mount helpers ─────────────────────────────────────────────────────────────

function makeQueryClient(pubkeyHex) {
  const qc = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, staleTime: Infinity },
    },
  });
  // Always set identity to an object (even for empty pubkey) so React Query
  // never calls queryFn = getIdentity (which would hit the unmocked IPC).
  // Component reads pubkeyHex = identity?.pubkey ?? "" — so { pubkey: "" }
  // gives pubkeyHex = "" (logged-out state).
  qc.setQueryData(["identity"], { pubkey: pubkeyHex });
  return qc;
}

function mountCard(qc) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const doRender = async () => {
    // ReportsTab calls useUsersBatchQuery which needs a get_users_batch handler.
    if (!ipcHandlers.get("get_users_batch")) {
      setIpcHandler("get_users_batch", () =>
        Promise.resolve({ profiles: {}, missing: [] }),
      );
    }
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(AdminConsoleSettingsCard),
          ),
        ),
      );
    });
  };
  const unmount = async () => {
    await act(async () => {
      root.unmount();
    });
    qc.clear();
    document.body.removeChild(container);
  };
  return { container, doRender, unmount };
}

/**
 * Mount AdminConsolePanel directly (not through the settings card).
 * Used for panel-level race tests (list, detail, attachment).
 */
function mountPanel({
  origin,
  pubkey,
  canMutate = true,
  initialTab = undefined,
}) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const qc = makeQueryClient(pubkey);
  const root = createRoot(container);
  const doRender = async ({ origin: o, pubkey: p } = { origin, pubkey }) => {
    // ReportsTab calls useUsersBatchQuery which needs QueryClientProvider +
    // CommunitiesProvider. Provide a default no-op handler so profile lookups
    // resolve without error when individual tests don't override get_users_batch.
    if (!ipcHandlers.get("get_users_batch")) {
      setIpcHandler("get_users_batch", () =>
        Promise.resolve({ profiles: {}, missing: [] }),
      );
    }
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: qc },
          React.createElement(
            CommunitiesProvider,
            null,
            React.createElement(AdminConsolePanel, {
              canMutate,
              origin: o,
              pubkey: p,
              ...(initialTab !== undefined ? { initialTab } : {}),
            }),
          ),
        ),
      );
    });
  };
  const unmount = async () => {
    await act(async () => {
      root.unmount();
    });
    qc.clear();
    document.body.removeChild(container);
  };
  return { container, doRender, unmount };
}

// Flush React effects and timers.
async function settle(ms = 20) {
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

afterEach(() => {
  clearIpcHandlers();
});

// ── parseImetaAttachments ─────────────────────────────────────────────────────

test("parseImetaAttachments: parses a well-formed imeta tag", () => {
  const sha256 = "a".repeat(64);
  const tags = [
    [
      "imeta",
      `url https://example.com/a.jpg`,
      `m image/jpeg`,
      `x ${sha256}`,
      "size 1234",
    ],
  ];
  const result = parseImetaAttachments(tags);
  assert.equal(result.length, 1);
  assert.equal(result[0].sha256, sha256);
  assert.equal(result[0].mime, "image/jpeg");
  assert.equal(result[0].size, 1234);
});

test("parseImetaAttachments: skips or rejects invalid inputs", () => {
  // Table of inputs that must produce an empty result. Retains every input
  // from the original standalone tests, including zero/negative sizes and
  // null/object/string non-array values.
  const sha256 = "a".repeat(64);
  const IMETA_REJECTION_ROWS = [
    {
      label: "non-imeta tags are skipped",
      tags: [
        ["p", "abc123"],
        ["e", "def456"],
      ],
    },
    {
      label: "uppercase x hash rejected",
      tags: [["imeta", `x ${"A".repeat(64)}`, "m image/png", "size 100"]],
    },
    {
      label: "hash shorter than 64 chars rejected",
      tags: [["imeta", `x ${"a".repeat(63)}`, "m image/png", "size 100"]],
    },
    {
      label: "hash longer than 64 chars rejected",
      tags: [["imeta", `x ${"a".repeat(65)}`, "m image/png", "size 100"]],
    },
    {
      label: "missing m field rejected",
      tags: [["imeta", `x ${"b".repeat(64)}`, "size 100"]],
    },
    {
      label: "missing size field rejected",
      tags: [["imeta", `x ${"c".repeat(64)}`, "m image/png"]],
    },
    {
      label: "zero size rejected",
      tags: [["imeta", `x ${sha256}`, "m image/png", "size 0"]],
    },
    {
      label: "negative size rejected",
      tags: [["imeta", `x ${sha256}`, "m image/png", "size -1"]],
    },
    { label: "null input returns empty array", tags: null },
    { label: "object input returns empty array", tags: {} },
    { label: "string input returns empty array", tags: "imeta" },
  ];
  for (const row of IMETA_REJECTION_ROWS) {
    assert.deepEqual(
      parseImetaAttachments(row.tags),
      [],
      `row must return []: ${row.label}`,
    );
  }
});

test("parseImetaAttachments: parses multiple imeta tags", () => {
  const sha1 = "e".repeat(64);
  const sha2 = "f".repeat(64);
  const tags = [
    ["imeta", `x ${sha1}`, "m image/png", "size 111"],
    ["imeta", `x ${sha2}`, "m image/jpeg", "size 222"],
  ];
  const result = parseImetaAttachments(tags);
  assert.equal(result.length, 2);
  assert.equal(result[0].sha256, sha1);
  assert.equal(result[1].sha256, sha2);
});

// ── Component-level session boundary and race tests ───────────────────────────
//
// Each test below mounts the production AdminConsoleSettingsCard (including
// AdminConsoleSettingsSession keyed by pubkeyHex) and drives Tauri IPC calls
// via deferred promises. These tests fail if the identity boundary or fences
// are removed from the production code.

test("authorized-logout-teardown: A's session is gone when pubkeyHex becomes empty", async () => {
  // Verifies the `pubkeyHex ? <AdminConsoleSettingsSession key=…> : null` render
  // gate in AdminConsoleSettingsCard. Drives the full authorized→logout transition:
  // mount with a real identity A, drive to authorized (input visible, panel rendered),
  // then switch pubkeyHex to "" and assert both input and panel are gone.
  //
  // Fails if the render gate is removed: after the transition to pubkeyHex="",
  // AdminConsoleSettingsSession re-mounts with empty pubkey and the input remains.
  //
  // Design: identical to identity-switch — act + qc.setQueryData + settle.
  // React Query's notifyManager fires onStoreChange via setTimeout(0), which
  // act() drains during the inner settle(). The MinimalDocument environment
  // handles this cleanly without the jsdom global scheduler side-effects.

  const pubkeyA = "a".repeat(64);
  const originA = "https://admin-a.example.com";

  setIpcHandler("get_admin_origin", (args) => {
    if (args?.expectedPubkey === pubkeyA) return Promise.resolve(originA);
    return Promise.resolve(null);
  });
  setIpcHandler("admin_probe", () =>
    Promise.resolve({ state: "nip98Authorized" }),
  );
  setIpcHandler("admin_list_reports", () => Promise.resolve([]));
  setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

  const qc = makeQueryClient(pubkeyA);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(50);

  // A is authorized — input and panel must be present.
  const inputA = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(inputA, "input must render for pubkeyA in authorized state");
  const panelA = container.querySelector("[data-testid='admin-console-panel']");
  assert.ok(panelA, "admin-console-panel must render when A is authorized");

  // Transition to logout — same pattern as identity-switch.
  await act(async () => {
    qc.setQueryData(["identity"], { pubkey: "" });
    await new Promise((r) => setTimeout(r, 25));
  });

  // After the transition: gate renders null, both input and panel must be gone.
  const inputAfter = container.querySelector(
    "[data-testid='admin-origin-input']",
  );
  const panelAfter = container.querySelector(
    "[data-testid='admin-console-panel']",
  );

  await unmount();

  assert.equal(
    inputAfter,
    null,
    "admin origin input must not render when pubkeyHex is empty — render gate missing",
  );
  assert.equal(
    panelAfter,
    null,
    "admin-console-panel must not render after logout — render gate missing",
  );
});
test("identity-switch: fresh session mounts with empty input on pubkey change", async () => {
  // Verifies the key-prop boundary. Without `key={pubkeyHex}`, React reuses
  // the component and A's origin state survives the switch to B.

  const pubkeyA = "a".repeat(64);
  const pubkeyB = "b".repeat(64);
  const originA = "https://admin-a.example.com";

  setIpcHandler("get_admin_origin", (args) => {
    if (args?.expectedPubkey === pubkeyA) return Promise.resolve(originA);
    return Promise.resolve(null);
  });
  setIpcHandler("admin_probe", () => Promise.resolve({ state: "disabled" }));

  const qc = makeQueryClient(pubkeyA);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(25);

  const inputA = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(inputA, "input must render for pubkeyA");
  assert.equal(
    inputA.value,
    originA,
    "input must show A's saved origin after mount",
  );

  // Switch to pubkeyB — key prop causes a full remount of AdminConsoleSettingsSession.
  // B has no saved origin, so the input must be empty.
  setIpcHandler("get_admin_origin", (args) => {
    if (args?.expectedPubkey === pubkeyB) return Promise.resolve(null);
    // Reject any call with A's pubkey — must not fire after the switch.
    return Promise.reject(new Error("unexpected pubkey after identity switch"));
  });

  await act(async () => {
    qc.setQueryData(["identity"], { pubkey: pubkeyB });
    await new Promise((r) => setTimeout(r, 25));
  });

  const inputB = container.querySelector("[data-testid='admin-origin-input']");
  assert.ok(inputB, "input must render for pubkeyB");
  assert.equal(
    inputB.value,
    "",
    "input must be empty for pubkeyB — key boundary ensures fresh state, not stale A origin",
  );
  await unmount();
});

test("storage-error surfaced: getAdminOrigin rejection shows error in UI", async () => {
  // Verifies the mount-effect catch sets `{ kind: 'error', message }`.
  // Removing error propagation from the catch (silent degrade) causes the
  // error text to not appear.

  const pubkey = "c".repeat(64);
  const errorMsg = "stored admin console origin is invalid (removed): bad json";
  setIpcHandler("get_admin_origin", () => Promise.reject(new Error(errorMsg)));

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(25);

  // The error or its key fragment must be visible in the rendered tree.
  const bodyText = container.textContent ?? "";
  const hasError =
    bodyText.includes("invalid") ||
    bodyText.includes("bad json") ||
    bodyText.includes("removed") ||
    bodyText.includes("admin console origin");
  assert.ok(
    hasError,
    `error from getAdminOrigin must appear in UI; body text: "${bodyText.slice(0, 300)}"`,
  );
  await unmount();
});

// origin-edit (abortAndResetProbe wired to onChange) is covered by
// adminConsolePanelSession.jsdom-test.mjs where fireEvent dispatches native
// events through React 19's container-level delegation.

// ── AdminConsolePanel race tests ──────────────────────────────────────────────
//
// These tests mount AdminConsolePanel directly (bypassing the settings card)
// and use deferred promises to simulate in-flight native requests. They verify
// the effect-local `active` flag cancellation in useAsyncLoad, the generation
// fence in AdminConsolePanel, and the loadGenRef cleanup in AttachmentViewer.

test("old-list-after-new-list: stale list result does not replace new list after pubkey change", async () => {
  // Verifies the effect-local `active` flag in useAsyncLoad.
  //
  // Scenario: panel renders with pubkeyA/originA → list query starts (deferred).
  // Before it resolves, panel re-renders with pubkeyB/originB → a new list
  // query starts. Then the old (A's) deferred resolves: the active flag in
  // A's effect closure is already false (effect re-ran with B's deps), so
  // A's result is discarded. Only B's result may commit.
  //
  // This test fails if useAsyncLoad's active-flag cleanup is removed, because
  // A's result would overwrite B's list state.

  const originA = "https://admin-a.example.com";
  const originB = "https://admin-b.example.com";
  const pubkeyA = "a".repeat(64);
  const pubkeyB = "b".repeat(64);

  const listDeferredA = deferred();
  const listDeferredB = deferred();

  // First call returns A's deferred; subsequent calls return B's.
  let callCount = 0;
  setIpcHandler("admin_list_reports", () => {
    callCount += 1;
    if (callCount === 1) return listDeferredA.promise;
    return listDeferredB.promise;
  });

  const { container, doRender, unmount } = mountPanel({
    origin: originA,
    pubkey: pubkeyA,
  });

  // Render with A — list query starts and stays pending (no settle; would hang).
  await act(async () => {
    await doRender({ origin: originA, pubkey: pubkeyA });
    await new Promise((r) => setTimeout(r, 0));
  });

  // Switch to B — triggers generation bump + effect cleanup (active = false for A).
  // Re-render causes the effect to re-run with B's deps.
  await act(async () => {
    await doRender({ origin: originB, pubkey: pubkeyB });
    await new Promise((r) => setTimeout(r, 0));
  });

  // Now resolve A's stale list with a distinct marker item.
  listDeferredA.resolve([
    {
      id: "00000000-0000-0000-0000-000000000001",
      communityId: "00000000-0000-0000-0000-000000000002",
      communityHost: "relay.example.com",
      reportEventId: "aabb",
      reporterPubkey: "ccdd",
      targetKind: "message",
      target: "eeff",
      reportType: "spam",
      status: "STALE-A-RESULT",
      createdAt: "2024-01-01T00:00:00Z",
    },
  ]);

  // Flush A's resolution — active is false so it must not commit.
  await act(async () => {
    await new Promise((r) => setTimeout(r, 30));
  });

  // A's stale result must not appear — active flag was false.
  const text = container.textContent ?? "";
  assert.ok(
    !text.includes("STALE-A-RESULT"),
    `stale list result from A must not appear after B renders; got: ${text.slice(0, 300)}`,
  );

  // Resolve B's list — this one is live.
  listDeferredB.resolve([
    {
      id: "00000000-0000-0000-0000-000000000003",
      communityId: "00000000-0000-0000-0000-000000000004",
      communityHost: "relay.example.com",
      reportEventId: "1122",
      reporterPubkey: "3344",
      targetKind: "message",
      target: "5566",
      reportType: "feedback",
      status: "LIVE-B-RESULT",
      createdAt: "2024-01-02T00:00:00Z",
    },
  ]);

  await act(async () => {
    await new Promise((r) => setTimeout(r, 30));
  });

  const textAfter = container.textContent ?? "";
  assert.ok(
    textAfter.includes("LIVE-B-RESULT"),
    `B's live list result must appear; got: ${textAfter.slice(0, 300)}`,
  );

  await unmount();
});

// detail-navigation and blob-leak-on-back-navigation (useAsyncLoad active flag,
// AttachmentViewer loadGenRef cleanup) are covered by
// adminConsolePanelSession.jsdom-test.mjs where fireEvent dispatches native
// events through React 19's container-level delegation.

// ── probe role/source gating — table-driven ──────────────────────────────
//
// Five rows cover the full probe-state → role-gate matrix. Each row shares
// the standard MinimalDocument mount; unique scheduler-sensitive assertions
// (disabled-probe-mounts-panel, probe-no-role) stay in this environment.
//
// Mutation evidence per row is preserved inline.

const PROBE_ROLE_ROWS = [
  {
    name: "disabled-probe-mounts-panel",
    desc: "admin-console-panel renders when probe state is disabled",
    pubkey: "f".repeat(64),
    savedOrigin: "https://admin.example.com",
    probeResult: { state: "disabled" },
    // Fails if render gate reverts to authorized-only (disabled state never mounts panel).
    check: (container) => {
      const panel = container.querySelector(
        "[data-testid='admin-console-panel']",
      );
      assert.ok(
        panel !== null,
        "admin-console-panel must mount when probe state is disabled — render gate missing",
      );
      const text = container.textContent ?? "";
      assert.ok(
        text.includes("Auth is disabled"),
        `disabled badge must remain visible; got: ${text.slice(0, 300)}`,
      );
    },
  },
  {
    name: "probe-role-source-badge",
    desc: "operator role and config source render in panel when probe returns them",
    pubkey: "b1".repeat(32),
    savedOrigin: "https://admin-role.example.com",
    probeResult: {
      state: "nip98Authorized",
      role: "operator",
      source: "config",
    },
    // Mutation: remove role/source from AdminProbeResult → badges absent → RED.
    check: (container) => {
      const text = container.textContent ?? "";
      assert.ok(
        text.includes("operator"),
        `role badge "operator" must render; got: ${text.slice(0, 300)}`,
      );
      assert.ok(
        text.includes("config"),
        `source badge "config" must render; got: ${text.slice(0, 300)}`,
      );
    },
  },
  {
    name: "probe-moderator-role",
    desc: "moderator role renders without staffing tab",
    pubkey: "c2".repeat(32),
    savedOrigin: "https://admin-mod.example.com",
    probeResult: { state: "nip98Authorized", role: "moderator", source: "db" },
    check: (container) => {
      const text = container.textContent ?? "";
      assert.ok(
        text.includes("moderator"),
        `role "moderator" must render; got: ${text.slice(0, 300)}`,
      );
      assert.equal(
        container.querySelector("[data-testid='admin-tab-staffing']"),
        null,
        "Staffing tab must not render for moderator role",
      );
    },
  },
  {
    name: "probe-operator-role",
    desc: "staffing tab renders for operator role",
    pubkey: "d3".repeat(32),
    savedOrigin: "https://admin-operator.example.com",
    probeResult: {
      state: "nip98Authorized",
      role: "operator",
      source: "config",
    },
    check: (container) => {
      const staffingTab = container.querySelector(
        "[data-testid='admin-tab-staffing']",
      );
      assert.ok(
        staffingTab !== null,
        "Staffing tab must render for operator role",
      );
    },
  },
  {
    name: "probe-no-role",
    desc: "disabled-mode panel renders without staffing tab",
    pubkey: "e4".repeat(32),
    savedOrigin: "https://admin-disabled.example.com",
    probeResult: { state: "disabled" },
    // Badge absence is not asserted here.
    check: (container) => {
      assert.ok(
        container.querySelector("[data-testid='admin-console-panel']") !== null,
        "panel must render in disabled mode",
      );
      assert.equal(
        container.querySelector("[data-testid='admin-tab-staffing']"),
        null,
        "Staffing tab must not render in disabled mode",
      );
    },
  },
];

for (const row of PROBE_ROLE_ROWS) {
  test(`${row.name}: ${row.desc}`, async () => {
    setIpcHandler("get_admin_origin", () => Promise.resolve(row.savedOrigin));
    setIpcHandler("admin_probe", () => Promise.resolve(row.probeResult));
    setIpcHandler("admin_list_reports", () => Promise.resolve([]));
    setIpcHandler("admin_list_feedback", () => Promise.resolve([]));

    const qc = makeQueryClient(row.pubkey);
    const { container, doRender, unmount } = mountCard(qc);
    await doRender();
    await settle(50);

    try {
      row.check(container);
    } finally {
      await unmount();
    }
  });
}

// ── denied badge copy button ──────────────────────────────────────────────

test("denied-badge-copy-button: copy button is present next to the denied pubkey", async () => {
  // Verifies item 2: the pubkey in the denied state is displayed alongside
  // a copy button (data-testid="admin-denied-pubkey-copy"), not just a
  // cursor-pointer select-all code block.

  const pubkey = "4".repeat(64);
  const savedOrigin = "https://admin-denied.example.com";

  setIpcHandler("get_admin_origin", () => Promise.resolve(savedOrigin));
  setIpcHandler("admin_probe", () => Promise.resolve({ state: "nip98Denied" }));

  const qc = makeQueryClient(pubkey);
  const { container, doRender, unmount } = mountCard(qc);
  await doRender();
  await settle(30);

  const pubkeyEl = container.querySelector(
    "[data-testid='admin-denied-pubkey']",
  );
  assert.ok(pubkeyEl !== null, "admin-denied-pubkey element must be present");
  assert.ok(
    pubkeyEl.textContent?.includes(pubkey),
    `denied pubkey element must contain the pubkey; got: ${pubkeyEl.textContent}`,
  );

  const copyBtn = container.querySelector(
    "[data-testid='admin-denied-pubkey-copy']",
  );
  assert.ok(
    copyBtn !== null,
    "admin-denied-pubkey-copy button must be present — copy-icon pattern missing",
  );

  await unmount();
});

// ── P1-2: applyAttachmentBudget — count and aggregate-byte limit ──────────

test("applyAttachmentBudget: items within count and byte limits pass through unchanged", () => {
  const items = [
    { sha256: "a".repeat(64), mime: "image/png", size: 100 },
    { sha256: "b".repeat(64), mime: "image/png", size: 200 },
  ];
  const { shown, truncated } = applyAttachmentBudget(items, 5, 1000);
  assert.equal(shown.length, 2);
  assert.equal(truncated, 0);
});

test("applyAttachmentBudget: excess attachments beyond MAX_COUNT are dropped", () => {
  // Build 7 attachments — limit is 5. Excess 2 must not be shown.
  // This is the regression Carl required: extra imeta entries on a feedback
  // item must NOT result in unbounded fetch fan-out.
  const items = Array.from({ length: 7 }, (_, i) => ({
    sha256: String(i).padStart(64, "0"),
    mime: "image/png",
    size: 100,
  }));
  const { shown, truncated } = applyAttachmentBudget(
    items,
    5,
    50 * 1024 * 1024,
  );
  assert.equal(
    shown.length,
    5,
    "only 5 attachments must be shown when 7 are present",
  );
  assert.equal(
    truncated,
    2,
    "2 excess attachments must be reported as truncated",
  );
  // The 6th and 7th items must not appear in shown — verifying the fetch
  // fan-out is bounded to the first 5.
  assert.ok(
    shown.every((a) => Number(a.sha256[0]) < 5),
    "shown items must be the first 5 by position",
  );
});

test("applyAttachmentBudget: aggregate byte limit drops items that would exceed the ceiling", () => {
  // 3 items totalling 30 MiB; cap is 25 MiB. Third item would push us over.
  const TEN_MIB = 10 * 1024 * 1024;
  const items = [
    { sha256: "a".repeat(64), mime: "image/png", size: TEN_MIB },
    { sha256: "b".repeat(64), mime: "image/png", size: TEN_MIB },
    { sha256: "c".repeat(64), mime: "image/png", size: TEN_MIB },
  ];
  const { shown, truncated } = applyAttachmentBudget(
    items,
    5,
    25 * 1024 * 1024,
  );
  assert.equal(shown.length, 2, "only 2 items fit within the 25 MiB ceiling");
  assert.equal(truncated, 1);
});

test("applyAttachmentBudget: empty list produces empty shown and zero truncated", () => {
  const { shown, truncated } = applyAttachmentBudget([], 5, 50 * 1024 * 1024);
  assert.equal(shown.length, 0);
  assert.equal(truncated, 0);
});

// ── P2 round-6 #2: reports-list always calls scope=all ───────────────────

test("reports-tab-scope-all: admin_list_reports IPC call includes scope=all", async () => {
  // Verifies that the ReportsTab always requests the full workflow queue via
  // scope=all, not the relay's escalated-only default (scope omitted).
  //
  // Mutation evidence: remove `{ scope: "all" }` from the listAdminReports
  // call → this test goes RED (captured query has no scope).

  const pubkey = "a9".repeat(32);
  const origin = "https://admin-scope.example.com";

  let capturedQuery = null;
  setIpcHandler("admin_list_reports", (args) => {
    capturedQuery = args?.query ?? null;
    return Promise.resolve([]);
  });

  const { doRender, unmount } = mountPanel({ origin, pubkey });
  await doRender();
  await settle(30);

  assert.ok(capturedQuery !== null, "admin_list_reports must have been called");
  assert.equal(
    capturedQuery?.scope,
    "all",
    `reports-tab IPC query must include scope="all"; got: ${JSON.stringify(capturedQuery)}`,
  );

  await unmount();
});
