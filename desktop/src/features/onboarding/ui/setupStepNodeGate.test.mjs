/**
 * Onboarding must not offer an install it cannot perform.
 *
 * `node_required` means Buzz has no npm on this platform, so an npm-backed
 * adapter install would reach a shell with no npm and fail. Doctor already
 * gates on `canAutoInstall && !nodeRequired` (`DoctorSettingsPanel.tsx`,
 * `RuntimeActions`); onboarding checked only `canAutoInstall`, so it rendered
 * an INSTALL button that invoked `install_acp_runtime` and failed.
 *
 * These tests mount the real exported `SetupStep` over a mocked Tauri IPC
 * boundary and assert on the boundary itself: the install command must not be
 * invoked in the Node-required state. Asserting rendered copy alone would not
 * close the defect — the button could still be wired to the installer.
 */

import assert from "node:assert/strict";
import test from "node:test";

// ── Minimal DOM shim (matches useLoadArchivedObserverEvents.test.mjs) ─────────

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
      this.tagName = tagName;
      this.children = [];
      this.childNodes = [];
      // ThemeProvider writes accent custom properties onto documentElement.
      this.style = {
        _props: new Map(),
        setProperty(name, value) {
          this._props.set(name, value);
        },
        removeProperty(name) {
          this._props.delete(name);
        },
        getPropertyValue(name) {
          return this._props.get(name) ?? "";
        },
      };
      this.nodeType = 1;
      this.parentNode = null;
      // react-dom writes nodeValue on text nodes during commit, so it must be
      // a plain writable property rather than the getter used for elements.
      this.nodeValue = null;
      this._attributes = new Map();
      this.classList = {
        add: () => {},
        remove: () => {},
        contains: () => false,
      };
    }
    get ownerDocument() {
      return globalThis.document;
    }
    get firstChild() {
      return this.children[0] ?? null;
    }
    get lastChild() {
      return this.children[this.children.length - 1] ?? null;
    }
    get nextSibling() {
      return null;
    }
    setAttribute(name, value) {
      this._attributes.set(name, String(value));
    }
    getAttribute(name) {
      return this._attributes.get(name) ?? null;
    }
    removeAttribute(name) {
      this._attributes.delete(name);
    }
    hasAttribute(name) {
      return this._attributes.has(name);
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
      return child;
    }
    insertBefore(newNode, refNode) {
      if (!refNode) return this.appendChild(newNode);
      const i = this.children.indexOf(refNode);
      if (i < 0) return this.appendChild(newNode);
      this.children.splice(i, 0, newNode);
      this.childNodes.splice(i, 0, newNode);
      newNode.parentNode = this;
      return newNode;
    }
    contains(node) {
      if (!node) return false;
      return this === node || this.children.some((c) => c?.contains?.(node));
    }
  }

  class MinimalDocument extends MinimalEventTarget {
    constructor() {
      super();
      this.nodeType = 9;
      this.documentElement = new MinimalNode("html");
    }
    createElement(tagName) {
      return new MinimalNode(tagName);
    }
    createElementNS(_ns, tagName) {
      return new MinimalNode(tagName);
    }
    createTextNode(value) {
      const n = new MinimalNode("#text");
      n.nodeValue = value;
      n.nodeType = 3;
      return n;
    }
    createComment(value) {
      const n = new MinimalNode("#comment");
      n.nodeValue = value;
      n.nodeType = 8;
      return n;
    }
    get body() {
      if (!this._body) this._body = this.createElement("body");
      return this._body;
    }
    get head() {
      if (!this._head) this._head = this.createElement("head");
      return this._head;
    }
    get activeElement() {
      return null;
    }
    querySelector() {
      return null;
    }
    contains(node) {
      return node != null;
    }
  }

  globalThis.document = new MinimalDocument();
  globalThis.HTMLIFrameElement = MinimalNode;
  globalThis.HTMLElement = MinimalNode;
  globalThis.Element = MinimalNode;
  globalThis.Node = MinimalNode;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";

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
  globalThis.requestAnimationFrame = (fn) => setTimeout(fn, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
  // ThemeProvider reads accent custom properties off the document element.
  globalThis.getComputedStyle = () => ({ getPropertyValue: () => "" });
  globalThis.matchMedia = () => ({
    matches: false,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
  });
  const store = new Map();
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      get length() {
        return store.size;
      },
      key: (i) => [...store.keys()][i] ?? null,
      getItem: (key) => store.get(key) ?? null,
      setItem: (key, value) => store.set(key, String(value)),
      removeItem: (key) => store.delete(key),
      clear: () => store.clear(),
    },
  });
}

installDOMShim();

// ── Tauri IPC interceptor ─────────────────────────────────────────────────────
//
// @tauri-apps/api/core calls window.__TAURI_INTERNALS__.invoke(cmd, args).
// Every invocation is recorded so tests can assert on which commands were
// reached — the install command's absence is the point of test 1.

/** @type {Array<{ cmd: string, args: unknown }>} */
let invocations = [];
/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
const ipcHandlers = new Map();

globalThis.__TAURI_INTERNALS__ = {
  invoke: (cmd, args) => {
    invocations.push({ cmd, args });
    const handler = ipcHandlers.get(cmd);
    if (handler) return handler(args);
    return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
  },
  transformCallback: () => Math.random(),
};

// ── Production imports (after shim, after IPC stub) ───────────────────────────

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { ThemeProvider } from "@/shared/theme/ThemeProvider";
import { SetupStep } from "./SetupStep.tsx";

// ── Fixtures ──────────────────────────────────────────────────────────────────

const INSTALL_COMMAND = "install_acp_runtime";
const OPEN_URL_COMMAND = "plugin:opener|open_url";
const NODEJS_URL = "https://nodejs.org";
const INSTRUCTIONS_URL = "https://example.test/install-guide";

/**
 * A raw (snake_case) catalog entry as `discover_acp_providers` returns it.
 *
 * `claude` is used because `ONBOARDING_RUNTIME_ORDER` only renders builtin
 * `claude`/`codex` cards today. The gate under test is per-entry data, so the
 * id is incidental — what matters is the `can_auto_install` / `node_required`
 * pair, which the backend can now produce for builtins and presets alike.
 */
function makeRawEntry({ canAutoInstall, nodeRequired }) {
  return {
    id: "claude",
    label: "Claude Code",
    avatar_url: "",
    availability: "adapter_missing",
    command: null,
    binary_path: null,
    default_args: [],
    mcp_command: null,
    install_hint: "Install the ACP adapter.",
    install_instructions_url: INSTRUCTIONS_URL,
    can_auto_install: canAutoInstall,
    requires_external_cli: false,
    underlying_cli_path: "/usr/local/bin/claude",
    node_required: nodeRequired,
    auth_status: { status: "unknown" },
    source: "builtin",
  };
}

/**
 * Mounts the real `SetupStep` with one catalog entry in the requested state.
 * Returns the container plus helpers to locate and click rendered buttons.
 */
async function mountSetupStep({ canAutoInstall, nodeRequired }) {
  invocations = [];
  ipcHandlers.clear();
  ipcHandlers.set("discover_acp_providers", async () => [
    makeRawEntry({ canAutoInstall, nodeRequired }),
  ]);
  ipcHandlers.set(OPEN_URL_COMMAND, async () => null);
  ipcHandlers.set(INSTALL_COMMAND, async () => ({
    success: true,
    steps: [],
    restarted_count: 0,
    failed_restart_count: 0,
  }));

  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const container = document.createElement("div");
  const root = createRoot(container);

  await act(async () => {
    root.render(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(
          ThemeProvider,
          null,
          React.createElement(SetupStep, {
            actions: { back: () => {}, next: () => {} },
            direction: "forward",
            onReadyRuntimeIdsChange: () => {},
          }),
        ),
      ),
    );
  });
  // Let the runtimes query resolve and commit.
  for (let i = 0; i < 4; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
  }

  return {
    findByTestId: (testId) => findByTestId(container, testId),
    runtimeAction: () => {
      const action = findRuntimeAction(container);
      assert.ok(action, "the runtime card must render exactly one action");
      return action;
    },
    click: async (node) => {
      await act(async () => {
        reactOnClick(node)({});
      });
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5));
      });
    },
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
      queryClient.clear();
    },
  };
}

/** Depth-first search for a node carrying `data-testid={testId}`. */
function findByTestId(node, testId) {
  if (node?.getAttribute?.("data-testid") === testId) return node;
  for (const child of node?.children ?? []) {
    const found = findByTestId(child, testId);
    if (found) return found;
  }
  return null;
}

/**
 * Finds the runtime card's single action button by its shared marker class,
 * without naming which branch rendered it.
 *
 * Locating the action this way is what lets the Node-gate test assert on
 * behavior: it clicks whatever the component decided to offer, so a regression
 * that swaps the Node.js affordance back to the installer is caught by the
 * install-command assertion rather than by a testid lookup returning null.
 */
function findRuntimeAction(node) {
  const className = node?.getAttribute?.("class") ?? "";
  if (className.includes("buzz-onboarding-runtime-setup")) return node;
  for (const child of node?.children ?? []) {
    const found = findRuntimeAction(child);
    if (found) return found;
  }
  return null;
}

/**
 * Reads the committed `onClick` handler off a DOM node. The shim dispatches no
 * synthetic events, so the React props stashed on the node are the seam for
 * driving a real click through the component's own handler.
 */
function reactOnClick(node) {
  const propsKey = Object.keys(node).find((key) =>
    key.startsWith("__reactProps$"),
  );
  assert.ok(propsKey, "node must carry committed React props");
  const onClick = node[propsKey].onClick;
  assert.equal(typeof onClick, "function", "node must have an onClick handler");
  return onClick;
}

function commandsInvoked() {
  return invocations.map((entry) => entry.cmd);
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/**
 * The defect this PR fixes. With `can_auto_install: true` AND
 * `node_required: true`, onboarding must route to Node.js instead of the
 * installer.
 *
 * The invocation assertion is the load-bearing one, and it is deliberately
 * made first: it clicks whatever affordance the card renders, so it fails on
 * the *behavior* (installer reached) rather than on a missing testid. Reverting
 * the gate in `SetupStep`'s `RuntimeStatus` renders the ordinary install button,
 * whose click invokes `install_acp_runtime` — verified red before commit.
 */
test("test_node_required_autoinstall_runtime_offers_node_and_never_invokes_installer", async () => {
  const view = await mountSetupStep({
    canAutoInstall: true,
    nodeRequired: true,
  });

  // Click the card's only action, whatever it is. A correct gate renders the
  // Node.js affordance; a missing gate renders the installer button here.
  await view.click(view.runtimeAction());

  assert.ok(
    !commandsInvoked().includes(INSTALL_COMMAND),
    `install_acp_runtime must never be invoked in the Node-required state; saw ${JSON.stringify(commandsInvoked())}`,
  );
  assert.deepEqual(
    invocations
      .filter((entry) => entry.cmd === OPEN_URL_COMMAND)
      .map((entry) => entry.args.url),
    [NODEJS_URL],
    "the action must send the user to nodejs.org instead",
  );
  assert.ok(
    view.findByTestId("onboarding-runtime-node-claude"),
    "Node.js affordance must be the rendered action",
  );
  assert.equal(
    view.findByTestId("onboarding-runtime-install-claude"),
    null,
    "the install button must not be offered when npm is absent",
  );

  await view.unmount();
});

/**
 * The unchanged happy path: npm is available, so the installer is reachable.
 * Pins that the new branch did not swallow the ordinary auto-install case.
 */
test("test_autoinstall_runtime_with_npm_available_invokes_installer", async () => {
  const view = await mountSetupStep({
    canAutoInstall: true,
    nodeRequired: false,
  });

  assert.ok(
    view.findByTestId("onboarding-runtime-install-claude"),
    "install button must render when npm is available",
  );
  assert.equal(
    view.findByTestId("onboarding-runtime-node-claude"),
    null,
    "the Node.js affordance must not appear when npm is available",
  );

  await view.click(view.runtimeAction());

  assert.deepEqual(
    invocations
      .filter((entry) => entry.cmd === INSTALL_COMMAND)
      .map((entry) => entry.args.runtimeId),
    ["claude"],
    "clicking must invoke the installer for this runtime",
  );

  await view.unmount();
});

/**
 * A docs-link-only runtime has no install plan at all, so `node_required` is
 * irrelevant to it. The new branch requires `canAutoInstall`, which keeps such
 * entries on their instructions link instead of diverting them to nodejs.org.
 */
test("test_docs_only_runtime_keeps_instructions_link_when_node_required", async () => {
  const view = await mountSetupStep({
    canAutoInstall: false,
    nodeRequired: true,
  });

  assert.ok(
    view.findByTestId("onboarding-runtime-instructions-claude"),
    "instructions link must render",
  );
  assert.equal(
    view.findByTestId("onboarding-runtime-node-claude"),
    null,
    "docs-link-only runtimes must not be diverted to nodejs.org",
  );

  await view.click(view.runtimeAction());

  assert.deepEqual(
    invocations
      .filter((entry) => entry.cmd === OPEN_URL_COMMAND)
      .map((entry) => entry.args.url),
    [INSTRUCTIONS_URL],
    "clicking must open the runtime's own install guide",
  );
  assert.ok(
    !commandsInvoked().includes(INSTALL_COMMAND),
    "a docs-link-only runtime must never invoke the installer",
  );

  await view.unmount();
});
