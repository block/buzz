/**
 * PR #6852 review: the pre-join read-only fence must also fail closed over
 * mutation surfaces opened BEFORE membership was revoked. Membership
 * notifications invalidate the channel queries live, so `channel.isMember`
 * can flip to false while the sidebar — and the "Manage agent access" dialog
 * it hosts — is open. The dialog's Save path dispatches the global
 * `update_managed_agent` mutation with no channel-membership guard of its
 * own, so the sidebar must close it on revocation and its Save must not be
 * dispatchable afterwards.
 *
 * Mutation proof: revert the `respondToDialogAgent` clamp/reset hunk in
 * MembersSidebar.tsx (pass `editRespondToAgent` straight through) and the
 * revocation test goes RED — the dialog stays mounted across the
 * isMember=false rerender.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  HTMLElement: dom.window.HTMLElement,
  HTMLIFrameElement: dom.window.HTMLIFrameElement,
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
// Copy all DOM-level globals from JSDOM that Radix Dialog/DropdownMenu
// focus/dismiss machinery references without a window. prefix.
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (
    !(key in globalThis) &&
    (key.startsWith("HTML") ||
      key.startsWith("SVG") ||
      key.startsWith("CSS") ||
      [
        "Element",
        "Document",
        "ShadowRoot",
        "DOMRect",
        "DOMRectReadOnly",
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
// getComputedStyle must be bound to dom.window or it throws "Illegal invocation".
globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
// JSDOM has no layout; Radix roving focus scrolls highlighted items into view.
dom.window.HTMLElement.prototype.scrollIntoView ??= () => {};

// Radix DismissableLayer and FocusScope dispatch plain objects via
// dispatchEvent for layer-coordination events. JSDOM's strict Event type
// validation throws on these; silently drop non-Event objects.
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function (event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

// ── Fixtures ─────────────────────────────────────────────────────────────────

const VIEWER = "a".repeat(64);
const BOT = "b".repeat(64);
const CHANNEL_ID = "11111111-2222-3333-4444-555555555555";

/** Raw snake_case managed agent as `list_managed_agents` returns over IPC. */
const RAW_BOT_AGENT = {
  pubkey: BOT,
  name: "Nimbus",
  persona_id: null,
  relay_url: "wss://relay.example",
  acp_command: "acp",
  agent_command: "agent",
  agent_args: [],
  mcp_command: null,
  turn_timeout_seconds: 60,
  idle_timeout_seconds: 60,
  max_turn_duration_seconds: 600,
  parallelism: 1,
  system_prompt: null,
  model: null,
  status: "stopped",
  pid: null,
  created_at: 1,
  updated_at: 1,
  last_started_at: null,
  last_stopped_at: null,
  last_exit_code: null,
  last_error: null,
  log_path: null,
  start_on_app_launch: false,
  backend: { type: "local" },
  backend_agent_id: null,
  respond_to: "owner-only",
  respond_to_allowlist: [],
};

const RAW_MEMBERS = {
  members: [
    {
      pubkey: VIEWER,
      role: "member",
      is_agent: false,
      joined_at: 1,
      display_name: "Viewer",
    },
    {
      pubkey: BOT,
      role: "bot",
      is_agent: true,
      joined_at: 1,
      display_name: "Nimbus",
    },
  ],
};

function memberChannel(isMember) {
  return {
    id: CHANNEL_ID,
    name: "orbit",
    channelType: "channel",
    visibility: "open",
    isMember,
    archivedAt: null,
  };
}

// ── Tauri IPC stub ────────────────────────────────────────────────────────────

let updateManagedAgentCalls = [];

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command, args) => {
    if (command.startsWith("plugin:")) return Promise.resolve(0);
    switch (command) {
      case "get_identity":
        return Promise.resolve({ pubkey: VIEWER, display_name: "Viewer" });
      case "get_channel_members":
        return Promise.resolve(RAW_MEMBERS);
      case "list_managed_agents":
        return Promise.resolve([RAW_BOT_AGENT]);
      case "list_relay_agents":
        return Promise.resolve([]);
      case "get_presence":
        return Promise.resolve({});
      case "get_users_batch":
        return Promise.resolve({ profiles: {}, missing: [] });
      case "search_users":
        return Promise.resolve({ users: [], next_cursor: null });
      case "list_archived_identities":
        return Promise.resolve({ archived: [] });
      case "get_my_relay_membership":
        return Promise.reject(new Error("relay returned 404 Not Found"));
      case "agent_access_owner_only":
        return Promise.resolve(false);
      case "update_managed_agent":
        updateManagedAgentCalls.push(args);
        return Promise.resolve({
          agent: RAW_BOT_AGENT,
          profile_sync_error: null,
        });
      default:
        return Promise.resolve(null);
    }
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;

// ── Deferred imports ──────────────────────────────────────────────────────────

let React,
  act,
  createRoot,
  QueryClient,
  QueryClientProvider,
  CommunitiesProvider,
  ThemeProvider,
  TooltipProvider,
  MembersSidebar;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ ThemeProvider } = await import("@/shared/theme/ThemeProvider.tsx"));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip.tsx"));
  ({ MembersSidebar } = await import("./MembersSidebar.tsx"));
});

afterEach(() => {
  updateManagedAgentCalls = [];
});

after(() => dom.window.close());

// ── Helpers ───────────────────────────────────────────────────────────────────

function sidebarTree(queryClient, channel) {
  return React.createElement(
    QueryClientProvider,
    { client: queryClient },
    React.createElement(
      ThemeProvider,
      null,
      React.createElement(
        TooltipProvider,
        null,
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(MembersSidebar, {
            channel,
            currentPubkey: VIEWER,
            onOpenChange: () => {},
            open: true,
          }),
        ),
      ),
    ),
  );
}

async function settle(ms = 50) {
  await act(async () => {
    await new Promise((r) => setTimeout(r, ms));
  });
}

function findSaveAccessButton() {
  return [...document.body.querySelectorAll("button")].find((button) =>
    button.textContent?.includes("Save access"),
  );
}

function accessDialogIsOpen() {
  return [...document.body.querySelectorAll('[role="dialog"]')].some((el) =>
    el.textContent?.includes("Manage agent access"),
  );
}

/**
 * Mount the sidebar as a joined member and open "Manage agent access" for the
 * managed bot through the real row menu. Returns render handles for the
 * caller to drive the membership transition.
 */
async function mountWithAccessDialogOpen() {
  // gcTime 0: the default 5-minute garbage-collection timers outlive
  // teardown and stall the node:test process for their full duration.
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { gcTime: 0 },
    },
  });
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  await act(async () => {
    root.render(sidebarTree(queryClient, memberChannel(true)));
  });
  await settle();

  // The member list virtualizes (zero-height in JSDOM), so filter by search —
  // the search branch renders matching rows directly.
  const searchInput = document.body.querySelector(
    '[data-testid="channel-management-search-users"]',
  );
  assert.ok(searchInput, "members search input must render");
  const setter = Object.getOwnPropertyDescriptor(
    dom.window.HTMLInputElement.prototype,
    "value",
  ).set;
  await act(async () => {
    setter.call(searchInput, "nim");
    searchInput.dispatchEvent(new dom.window.Event("input", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 0));
  });
  await settle();

  const menuTrigger = document.body.querySelector(
    `[data-testid="sidebar-member-menu-${BOT}"]`,
  );
  assert.ok(menuTrigger, "bot row actions menu must render for a member");
  await act(async () => {
    menuTrigger.dispatchEvent(
      new dom.window.MouseEvent("pointerdown", { bubbles: true, button: 0 }),
    );
    await new Promise((r) => setTimeout(r, 0));
  });
  await settle();

  const editItem = document.body.querySelector(
    `[data-testid="sidebar-edit-respond-to-${BOT}"]`,
  );
  assert.ok(editItem, "Manage agent access menu item must render");
  await act(async () => {
    editItem.dispatchEvent(
      new dom.window.MouseEvent("click", { bubbles: true }),
    );
    await new Promise((r) => setTimeout(r, 0));
  });
  await settle();

  assert.ok(accessDialogIsOpen(), "Manage agent access dialog must open");

  return {
    queryClient,
    container,
    root,
    async rerender(channel) {
      await act(async () => {
        root.render(sidebarTree(queryClient, channel));
      });
      await settle();
    },
    async teardown() {
      await act(async () => {
        root.unmount();
      });
      container.remove();
      queryClient.clear();
    },
  };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

test("control: Save dispatches update_managed_agent while still a member", async () => {
  const surface = await mountWithAccessDialogOpen();

  const saveButton = findSaveAccessButton();
  assert.ok(saveButton, "Save access button must render for a member");
  await act(async () => {
    saveButton.dispatchEvent(
      new dom.window.MouseEvent("click", { bubbles: true }),
    );
    await new Promise((r) => setTimeout(r, 0));
  });
  await settle();

  assert.equal(
    updateManagedAgentCalls.length,
    1,
    "Save must dispatch exactly one update_managed_agent while a member",
  );

  await surface.teardown();
});

test("membership revocation closes the open access dialog and Save cannot dispatch", async () => {
  const surface = await mountWithAccessDialogOpen();

  const staleSaveButton = findSaveAccessButton();
  assert.ok(
    staleSaveButton,
    "Save access button must render before revocation",
  );

  // Membership revoked mid-session: notifications invalidate the channel
  // queries and isMember flips to false with the sidebar still open.
  await surface.rerender(memberChannel(false));

  assert.equal(
    accessDialogIsOpen(),
    false,
    "revocation must close the already-open Manage agent access dialog",
  );
  assert.equal(
    findSaveAccessButton(),
    undefined,
    "no Save access button may remain reachable after revocation",
  );

  // Even a click landing on the stale pre-revocation button must not reach
  // the mutation.
  await act(async () => {
    staleSaveButton.dispatchEvent(
      new dom.window.MouseEvent("click", { bubbles: true }),
    );
    await new Promise((r) => setTimeout(r, 0));
  });
  await settle();
  assert.equal(
    updateManagedAgentCalls.length,
    0,
    "update_managed_agent must not fire after revocation",
  );

  // The stale selection is cleared, not merely hidden: re-joining later must
  // not spontaneously reopen the dialog.
  await surface.rerender(memberChannel(true));
  assert.equal(
    accessDialogIsOpen(),
    false,
    "re-join must not resurrect the pre-revocation dialog",
  );

  await surface.teardown();
});
