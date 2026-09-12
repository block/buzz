/**
 * Catalog-browse wiring regression for the publisher-avatar IP leak.
 *
 * `ProfileAvatarUntrusted.test.mjs` proves the component guard in isolation,
 * but it never renders `CommunityCatalogDialog` — so deleting `untrusted` from
 * any of the three browse sites (persona sidebar row, persona detail header,
 * team member row) would leave that test green while restoring the exact leak
 * Carl flagged: opening Discover Teams fires image requests at up to 64
 * publisher-controlled hosts, handing the viewer's IP and browse timing away.
 *
 * This test mounts the real dialog with publisher URLs on every avatar-bearing
 * projection, drives selection through all three sites, and asserts zero
 * HTTP(S) `Image.src` assignments — the actual network trigger Radix fires.
 * Removing `untrusted` from any single site turns it RED.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Radix's AvatarImage probes load status by assigning `.src` on a detached
// `new window.Image()`; that assignment is the network request. Spy on it so
// the test observes the fetch itself rather than post-load DOM (which never
// mounts under jsdom because the probe never fires `load`).
const imageSrcAssignments = [];

class SpyImage {
  constructor() {
    this.complete = false;
    this.naturalWidth = 0;
    this._src = "";
  }
  addEventListener() {}
  removeEventListener() {}
  set src(value) {
    this._src = value;
    imageSrcAssignments.push(value);
  }
  get src() {
    return this._src;
  }
}

Object.assign(globalThis, {
  HTMLElement: dom.window.HTMLElement,
  IS_REACT_ACT_ENVIRONMENT: true,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: class {
    observe() {}
    unobserve() {}
    disconnect() {}
  },
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
});
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
dom.window.ResizeObserver = globalThis.ResizeObserver;
dom.window.matchMedia ??= (query) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
});
globalThis.matchMedia = dom.window.matchMedia;
// Radix Dialog's focus/dismiss machinery references many DOM globals without a
// window. prefix; copy them in bulk to avoid per-global whack-a-mole.
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (
    !(key in globalThis) &&
    (key.startsWith("HTML") ||
      key.startsWith("SVG") ||
      key.startsWith("CSS") ||
      [
        "Node",
        "NodeFilter",
        "NodeList",
        "NamedNodeMap",
        "Event",
        "CustomEvent",
        "MouseEvent",
        "KeyboardEvent",
        "FocusEvent",
        "InputEvent",
        "PointerEvent",
        "TouchEvent",
        "WheelEvent",
        "EventTarget",
        "Text",
        "Comment",
        "DocumentFragment",
        "Range",
        "Selection",
        "getComputedStyle",
        "IntersectionObserver",
        "ResizeObserver",
      ].includes(key))
  ) {
    const val = dom.window[key];
    if (val !== undefined) globalThis[key] = val;
  }
}
globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);

// Radix DismissableLayer/FocusScope dispatch plain objects; JSDOM's strict
// Event validation throws on them. Drop non-Event objects so the dialog renders
// without throwing from effects; real Event delivery is unaffected.
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function (event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

dom.window.Image = SpyImage;
globalThis.Image = SpyImage;

// The owner-label batch query would cross the Tauri IPC boundary; resolve it to
// an empty profile set so the detail panes render without an unmocked reject.
globalThis.__TAURI_INTERNALS__ = {
  invoke: (command) => {
    if (command === "get_users_batch") {
      return Promise.resolve({ profiles: {}, missing: [] });
    }
    return Promise.reject(new Error(`unmocked: ${command}`));
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let CommunitiesProvider;
let TooltipProvider;
let ThemeProvider;
let CommunityCatalogDialog;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip.tsx"));
  ({ ThemeProvider } = await import("@/shared/theme/ThemeProvider.tsx"));
  ({ CommunityCatalogDialog } = await import("./CommunityCatalogDialog.tsx"));
});

afterEach(() => {
  imageSrcAssignments.length = 0;
});

after(() => dom.window.close());

// Distinct publisher hosts per site so a RED assertion names the leaking one.
const PERSONA_AVATAR = "https://persona.attacker.example/beacon.png";
const MEMBER_AVATAR = "https://member.attacker.example/beacon.png";

const networkAssignments = () =>
  imageSrcAssignments.filter((src) => /^https?:/i.test(src));

function catalogPersona() {
  return {
    id: "persona-1",
    displayName: "Mallory",
    avatarUrl: PERSONA_AVATAR,
    systemPrompt: "Do things.",
    runtime: "goose",
    model: "claude",
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: false,
    shared: true,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    // Marks a foreign catalog entry so PersonaCatalogDetail resolves an owner
    // label — exercises the detail header (site 735) as a browsed row.
    catalogSource: { ownerPubkey: "a".repeat(64), teamDTag: "", isOwn: false },
  };
}

function catalogTeam() {
  return {
    eventId: "ev-1",
    ownerPubkey: "b".repeat(64),
    teamDTag: "crew",
    name: "Crew",
    description: "A crew.",
    instructions: null,
    members: [
      {
        memberKey: "m-1",
        displayName: "Eve",
        systemPrompt: "Review.",
        avatarUrl: MEMBER_AVATAR,
        runtime: "goose",
        model: "claude",
        provider: null,
      },
    ],
    isOwn: false,
    localTeam: null,
  };
}

async function mountDialog() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);

  const tree = () =>
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(
        ThemeProvider,
        null,
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            TooltipProvider,
            null,
            React.createElement(CommunityCatalogDialog, {
              createContent: () => React.createElement("div", null, "create"),
              onImportFile: () => {},
              personas: [catalogPersona()],
              personasError: null,
              personasLoading: false,
              personasPending: false,
              feedbackErrorMessage: null,
              feedbackNoticeMessage: null,
              onClearFeedback: () => {},
              onSelectPersona: () => {},
              teams: [catalogTeam()],
              teamsError: null,
              teamsLoading: false,
              teamsAdding: false,
              onAddTeam: () => {},
              open: true,
              // "agents" so the dialog does not auto-select the first team; the
              // test drives each selection explicitly.
              preferSection: "agents",
              onOpenChange: () => {},
            }),
          ),
        ),
      ),
    );

  await act(async () => {
    root.render(tree());
  });
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  return { root, container, client };
}

async function clickTestId(testId) {
  const el = dom.window.document.querySelector(`[data-testid="${testId}"]`);
  assert.ok(el, `expected element ${testId}`);
  await act(async () => {
    el.dispatchEvent(
      new dom.window.MouseEvent("click", { bubbles: true, cancelable: true }),
    );
    await new Promise((r) => setTimeout(r, 0));
  });
}

test("catalog browse fires no publisher image request across all three avatar sites", async () => {
  const { root, container, client } = await mountDialog();

  // Site 1 — persona sidebar row is rendered on open.
  assert.deepEqual(
    networkAssignments(),
    [],
    "persona sidebar avatar leaked a network request",
  );

  // Site 2 — persona detail header.
  await clickTestId("community-catalog-agent-persona-1");
  assert.deepEqual(
    networkAssignments(),
    [],
    "persona detail avatar leaked a network request",
  );

  // Site 3 — team member row (avatar is in the always-visible expander button).
  await clickTestId(`community-catalog-team-${"b".repeat(64)}:crew`);
  assert.deepEqual(
    networkAssignments(),
    [],
    "team member avatar leaked a network request",
  );

  await act(async () => {
    root.unmount();
  });
  container.remove();
  client.clear();
});
