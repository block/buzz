/**
 * Rendered coverage for the hidden-DM Inbox reopen affordance (Jude P2).
 *
 * The reopen flow only exists once the REAL useHiddenDmInboxNavigation hook is
 * wired to the REAL InboxListPane / InboxDetailPane, exactly as HomeView wires
 * it. A pure action test (hiddenDmInboxAction.test.mjs) proves the command
 * mechanics; it cannot prove the rendered contract Jude flagged:
 *
 *   - a single open_dm is issued per activation (pointer, context-menu, keyboard),
 *   - a duplicate activation while a reopen is pending is suppressed,
 *   - navigation is withheld until the reopen resolves (no premature nav),
 *   - a perceivable, immediately announceable pending state renders with
 *     role="status" and no aria-busy suppression,
 *   - a failure surfaces an actionable, keyboard-reachable Retry, and
 *   - a successful retry finally navigates.
 *
 * Only the Tauri IPC boundary (open_dm, get_channel_members, get_channels,
 * get_identity, get_users_batch) and the TipTap MessageComposer are stubbed;
 * the hook, the panes, and the action all run their production code.
 */

import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

// MessageComposer mounts TipTap, which never releases jsdom handles and hangs
// the node:test process. Stub it to a null component so InboxDetailPane can
// prove its reopen wiring without pulling the editor in.
registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "@/features/messages/ui/MessageComposer") {
      return { shortCircuit: true, url: "buzz-inbox-stub:MessageComposer" };
    }
    if (specifier === "@/features/settings/UpdateIndicator") {
      return { shortCircuit: true, url: "buzz-inbox-stub:UpdateIndicator" };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-inbox-stub:MessageComposer") {
      return {
        format: "module",
        shortCircuit: true,
        source: "export const MessageComposer = () => null;\n",
      };
    }
    if (url === "buzz-inbox-stub:UpdateIndicator") {
      // The real UpdateIndicator pulls in UpdaterProvider's background-check
      // setInterval, which keeps the event loop alive past the test. It has
      // nothing to do with the reopen contract, so stub it to a null render.
      return {
        format: "module",
        shortCircuit: true,
        source: "export const UpdateIndicator = () => null;\n",
      };
    }
    return nextLoad(url, context);
  },
});

const SELF = "1".repeat(64);
const PEER = "2".repeat(64);
const HIDDEN_DM_ID = "hidden-dm-channel";
const SOURCE_EVENT_ID = "e".repeat(64);
const RELAY_URL = "wss://relay.example";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

class NoopObserver {
  disconnect() {}
  observe() {}
  unobserve() {}
}

// Neither pane nor the reopen path needs a live relay socket; a real
// (undici) WebSocket would open a connection to the seeded relay URL and
// leak an open handle that keeps the test process alive. Stub it to a
// non-connecting shell.
class NoopWebSocket {
  close() {}
  send() {}
  addEventListener() {}
  removeEventListener() {}
}
globalThis.WebSocket = NoopWebSocket;
dom.window.WebSocket = NoopWebSocket;

Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  IntersectionObserver: NoopObserver,
  MutationObserver: dom.window.MutationObserver,
  ResizeObserver: NoopObserver,
  document: dom.window.document,
  localStorage: dom.window.localStorage,
  self: dom.window,
  window: dom.window,
});
// Bulk-copy DOM constructors Radix / React reference without a window prefix.
for (const key of Object.getOwnPropertyNames(dom.window)) {
  if (
    !(key in globalThis) &&
    (key.startsWith("HTML") ||
      key.startsWith("SVG") ||
      [
        "Element",
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
        "Text",
        "Comment",
        "DocumentFragment",
        "Range",
        "Selection",
      ].includes(key))
  ) {
    const value = dom.window[key];
    if (value !== undefined) globalThis[key] = value;
  }
}
Object.defineProperty(globalThis, "navigator", {
  configurable: true,
  value: dom.window.navigator,
  writable: true,
});
globalThis.getComputedStyle = dom.window.getComputedStyle.bind(dom.window);
dom.window.matchMedia = () => ({
  matches: false,
  addEventListener() {},
  removeEventListener() {},
});
globalThis.matchMedia = dom.window.matchMedia;
dom.window.requestAnimationFrame = (callback) => setTimeout(callback, 0);
dom.window.cancelAnimationFrame = (id) => clearTimeout(id);
globalThis.requestAnimationFrame = dom.window.requestAnimationFrame;
globalThis.cancelAnimationFrame = dom.window.cancelAnimationFrame;

// Radix DismissableLayer/FocusScope dispatch plain objects; JSDOM's strict
// Event validation throws on them. Drop non-Event objects silently.
const _origDispatch = dom.window.EventTarget.prototype.dispatchEvent;
dom.window.EventTarget.prototype.dispatchEvent = function dispatchEvent(event) {
  if (!(event instanceof dom.window.Event)) return false;
  return _origDispatch.call(this, event);
};
globalThis.EventTarget = dom.window.EventTarget;

// JSDOM does not perform a native <button>'s default action, so a focused
// button never turns an Enter/Space keydown into a click the way every browser
// does. Install that documented default action at the document level so a
// genuine keyboard event drives real activation instead of a synthetic click.
dom.window.document.addEventListener("keydown", (event) => {
  if (event.defaultPrevented) return;
  const target = event.target;
  if (
    target instanceof dom.window.HTMLButtonElement &&
    !target.disabled &&
    (event.key === "Enter" || event.key === " ")
  ) {
    target.dispatchEvent(
      new dom.window.MouseEvent("click", { bubbles: true, cancelable: true }),
    );
  }
});

// The virtualized Inbox list measures its caller-owned scroll container; JSDOM
// reports zero layout, so force a nonzero box for every element.
Object.defineProperty(dom.window.HTMLElement.prototype, "offsetHeight", {
  configurable: true,
  get() {
    return 600;
  },
});
Object.defineProperty(dom.window.HTMLElement.prototype, "offsetWidth", {
  configurable: true,
  get() {
    return 400;
  },
});
// JSDOM does not implement scrollIntoView; InboxDetailPane's anchored-scroll
// layout effect calls it on mount. A no-op keeps the detail pane mountable.
dom.window.HTMLElement.prototype.scrollIntoView = function scrollIntoView() {};

// ── Tauri IPC stub ────────────────────────────────────────────────────────────

/** Resolves open_dm; each test installs the behavior it needs. */
let openDmHandler = async () => ({ id: HIDDEN_DM_ID, channel_type: "dm" });
let openDmCalls = 0;

globalThis.__TAURI_INTERNALS__ = {
  invoke: (command, args) => {
    if (command === "get_identity") {
      return Promise.resolve({ pubkey: SELF, display_name: "Me" });
    }
    if (command === "get_channels") {
      // The hidden DM is NOT in the visible list — this is what forces a reopen.
      return Promise.resolve({ hash: "h", channels: [], last_messages: {} });
    }
    if (command === "get_channel_members") {
      return Promise.resolve({
        members: [
          { pubkey: SELF, role: "member", is_agent: false },
          { pubkey: PEER, role: "member", is_agent: false },
        ],
      });
    }
    if (command === "open_dm") {
      openDmCalls += 1;
      return openDmHandler(args);
    }
    if (command === "get_users_batch") {
      return Promise.resolve({ profiles: {}, missing: [] });
    }
    if (command === "get_open_channel_directory") return Promise.resolve([]);
    if (command.startsWith("plugin:event|")) return Promise.resolve(0);
    return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
  },
  transformCallback: () => 1,
};
dom.window.__TAURI_INTERNALS__ = globalThis.__TAURI_INTERNALS__;
globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ =
  globalThis.__TAURI_EVENT_PLUGIN_INTERNALS__;

function seedCommunity() {
  window.localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-a",
        name: "Community A",
        relayUrl: RELAY_URL,
        pubkey: SELF,
        addedAt: "2026-01-01T00:00:00Z",
      },
    ]),
  );
  window.localStorage.setItem("buzz-active-community-id", "community-a");
}

const HIDDEN_DM_ITEM = {
  avatarUrl: null,
  conversationId: HIDDEN_DM_ID,
  id: SOURCE_EVENT_ID,
  item: {
    id: SOURCE_EVENT_ID,
    kind: 9,
    pubkey: PEER,
    content: "ping",
    createdAt: 1_700_000_000,
    channelId: HIDDEN_DM_ID,
    channelName: "peer",
    channelType: "dm",
    tags: [],
    category: "activity",
  },
  categories: ["activity"],
  categoryLabel: "Activity",
  channelLabel: "peer",
  fullTimestampLabel: "Jan 1, 2026, 12:00 AM",
  groupItems: [],
  isActionRequired: false,
  latestActivityAt: 1_700_000_000,
  mentionNames: [],
  preview: "ping",
  senderLabel: "Peer",
  subject: "",
  timestampLabel: "12:00 AM",
  unreadCount: 1,
};

// A second, already-open conversation used as the selected detail while the
// hidden DM row fails to reopen — so the detail pane's Retry belongs to a
// different channel and cannot cover the failed row.
const OTHER_CHANNEL_ID = "other-open-channel";
const OTHER_EVENT_ID = "f".repeat(64);
const OTHER_ITEM = {
  ...HIDDEN_DM_ITEM,
  conversationId: OTHER_CHANNEL_ID,
  id: OTHER_EVENT_ID,
  item: {
    ...HIDDEN_DM_ITEM.item,
    id: OTHER_EVENT_ID,
    channelId: OTHER_CHANNEL_ID,
    channelName: "other",
    channelType: "channel",
  },
  channelLabel: "other",
  senderLabel: "Other",
};

let React;
let act;
let createRoot;
let QueryClient;
let QueryClientProvider;
let CommunitiesProvider;
let useHiddenDmInboxNavigation;
let InboxListPane;
let InboxDetailPane;
let RouterContextProvider;
let TooltipProvider;
let router;

before(async () => {
  ({ default: React, act } = await import("react"));
  ({ createRoot } = await import("react-dom/client"));
  ({ TooltipProvider } = await import("@/shared/ui/tooltip.tsx"));
  const {
    RouterContextProvider: RouterCtx,
    createMemoryHistory,
    createRootRoute,
    createRouter,
  } = await import("@tanstack/react-router");
  RouterContextProvider = RouterCtx;
  // A minimal in-memory router only supplies the router context that
  // useAppNavigation reads (useRouter/useLocation/useNavigate). The reopen
  // path never calls goChannel, so no routes need to resolve — the context
  // just has to exist, exactly as it does under the real app shell.
  const rootRoute = createRootRoute();
  router = createRouter({
    routeTree: rootRoute,
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  ({ QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  ));
  ({ CommunitiesProvider } = await import(
    "@/features/communities/useCommunities.tsx"
  ));
  ({ useHiddenDmInboxNavigation } = await import(
    "@/features/home/useHiddenDmInboxNavigation.ts"
  ));
  ({ InboxListPane } = await import("./InboxListPane.tsx"));
  ({ InboxDetailPane } = await import("./InboxDetailPane.tsx"));
});

after(() => dom.window.close());

/**
 * Mounts the two panes wired to the real hook exactly as HomeView does. A
 * router is not needed: the reopen path terminates at onOpenContext, which we
 * capture, and goChannel is only reached via handleOpenDm (not exercised here).
 */
async function mountInbox(options = {}) {
  const { items = [HIDDEN_DM_ITEM], selectedItem = HIDDEN_DM_ITEM } = options;
  seedCommunity();
  openDmCalls = 0;
  const navigations = [];
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { gcTime: 0 },
    },
  });
  client.setQueryData(["identity"], { pubkey: SELF, displayName: "Me" });

  function Surface() {
    const nav = useHiddenDmInboxNavigation({
      availableChannelIds: React.useMemo(() => new Set(), []),
      currentPubkey: SELF,
      onOpenContext: (channelId, messageId, threadRootId) =>
        navigations.push([channelId, messageId, threadRootId ?? null]),
      selectedItem,
    });
    return React.createElement(
      React.Fragment,
      null,
      React.createElement(InboxListPane, {
        activeDraftCount: 0,
        draftItems: [],
        doneSet: new Set(),
        filter: "all",
        items,
        onFilterChange() {},
        onDeleteDraft() {},
        onMarkRead() {},
        onMarkUnread() {},
        onOpenDirect: nav.handleOpenDirect,
        isReopenPending: nav.isReopenPending,
        isReopenErrored: nav.isReopenErrored,
        onRemindLater() {},
        onSelect() {},
        onSelectDraft() {},
        onSelectReminder() {},
        onUnreadOnlyChange() {},
        selectedConversationId: selectedItem.conversationId,
        selectedDraftKey: null,
        dueReminderCount: 0,
        reminders: [],
        selectedReminderId: null,
        unreadOnly: false,
      }),
      React.createElement(InboxDetailPane, {
        canDelete: false,
        canOpenChannel: nav.canOpenSelected,
        canReply: false,
        channel: null,
        currentPubkey: SELF,
        editTargetId: null,
        item: selectedItem,
        selectedEventId: selectedItem.id,
        onDelete() {},
        onDeleteMessage() {},
        onEditTargetChange() {},
        onEditSave: async () => {},
        onRequestEmptyEditDelete() {},
        onManageChannel() {},
        onOpenContext: nav.handleOpenSelectedContext,
        reopenPending: nav.isReopenPending(selectedItem.item.channelId),
        reopenErrored: nav.isReopenErrored(selectedItem.item.channelId),
        onSendReply: async () => {},
      }),
    );
  }

  const container = dom.window.document.createElement("div");
  dom.window.document.body.appendChild(container);
  const root = createRoot(container);
  const tree = () =>
    React.createElement(
      RouterContextProvider,
      { router },
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(
          CommunitiesProvider,
          null,
          React.createElement(
            TooltipProvider,
            null,
            React.createElement(Surface),
          ),
        ),
      ),
    );
  await act(async () => {
    root.render(tree());
  });
  // A second commit lets VirtualizedList's layout effect re-run once the
  // caller-owned scroll container is attached, so the list rows measure and
  // render (child layout effects fire before the parent ref is populated).
  await act(async () => {
    root.render(tree());
  });
  await settle();

  return {
    container,
    navigations,
    async settle() {
      await settle();
    },
    async unmount() {
      await act(async () => {
        root.unmount();
      });
      // Drop every cached query/mutation and its gc timer. Without an explicit
      // clear, the identity/channel/member queries this flow issues leave
      // gcTime timers ref'd in the loop and the test process never exits.
      client.getQueryCache().clear();
      client.getMutationCache().clear();
      client.clear();
      client.unmount();
      container.remove();
    },
  };

  async function settle() {
    for (let i = 0; i < 6; i++) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 5));
      });
    }
  }
}

function click(element) {
  element.dispatchEvent(new dom.window.MouseEvent("click", { bubbles: true }));
}

test("pointer activation issues one reopen, shows pending, then navigates", async () => {
  const inbox = await mountInbox();
  try {
    const openButton = inbox.container.querySelector(
      '[data-testid="home-inbox-item-' +
        SOURCE_EVENT_ID +
        '"] [aria-label="Open in channel"]',
    );
    assert.ok(openButton, "row open-in-channel action must render");

    // Hold the reopen open so the pending state is observable before it settles.
    let release;
    openDmHandler = () =>
      new Promise((resolve) => {
        release = () => resolve({ id: HIDDEN_DM_ID, channel_type: "dm" });
      });

    await act(async () => {
      click(openButton);
    });
    // Pending state is perceivable with the right AX semantics.
    const status = inbox.container.querySelector(
      '[data-testid="home-inbox-reopen-status"]',
    );
    assert.ok(status, "a persistent reopen status region must render");
    assert.equal(status.getAttribute("role"), "status");
    // The pending status must be immediately announceable: aria-busy on the
    // live region would defer the update, and the region unmounts on success
    // before ever committing aria-busy=false, so "Reopening…" could go
    // unannounced (Jude's P2). Assert the region carries no busy suppression.
    assert.equal(status.getAttribute("aria-busy"), null);
    assert.match(status.textContent, /Reopening/);
    assert.equal(inbox.navigations.length, 0, "no premature navigation");

    await act(async () => {
      release();
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    await inbox.settle();

    assert.equal(openDmCalls, 1, "exactly one open_dm issued");
    assert.deepEqual(inbox.navigations, [
      [HIDDEN_DM_ID, SOURCE_EVENT_ID, null],
    ]);
  } finally {
    await inbox.unmount();
  }
});

test("context-menu activation shows pending, suppresses duplicates, then navigates", async () => {
  const inbox = await mountInbox();
  try {
    // Hold the reopen so the context-menu path's pending state is observable
    // and a second activation during pending can be proven a no-op.
    let release;
    openDmHandler = () =>
      new Promise((resolve) => {
        release = () => resolve({ id: HIDDEN_DM_ID, channel_type: "dm" });
      });
    const trigger = inbox.container.querySelector(
      '[data-testid="home-inbox-item-' + SOURCE_EVENT_ID + '"]',
    );
    assert.ok(trigger, "the row context-menu trigger must render");
    await act(async () => {
      trigger.dispatchEvent(
        new dom.window.MouseEvent("contextmenu", { bubbles: true }),
      );
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    const openItem = [
      ...dom.window.document.querySelectorAll('[role="menuitem"]'),
    ].find((node) => /Open in channel/.test(node.textContent));
    assert.ok(openItem, "context menu must offer Open in channel");
    await act(async () => {
      click(openItem);
    });

    // Pending is perceivable for the context-menu path too, and no navigation
    // has happened yet.
    const status = inbox.container.querySelector(
      '[data-testid="home-inbox-reopen-status"]',
    );
    assert.ok(status, "a persistent reopen status region must render");
    assert.equal(status.getAttribute("role"), "status");
    assert.equal(status.getAttribute("aria-busy"), null);
    assert.match(status.textContent, /Reopening/);
    assert.equal(inbox.navigations.length, 0, "no premature navigation");
    assert.equal(openDmCalls, 1, "one open_dm issued from the menu");

    // A second activation of the same row while the first is pending must be
    // suppressed — canOpen is false during pending, so no duplicate command.
    const rowOpenButton = inbox.container.querySelector(
      '[data-testid="home-inbox-item-' +
        SOURCE_EVENT_ID +
        '"] [aria-label="Reopening…"]',
    );
    assert.ok(
      rowOpenButton,
      "the row open action reflects pending via its label",
    );
    assert.equal(
      rowOpenButton.disabled,
      true,
      "the open action is disabled while a reopen is pending",
    );
    await act(async () => {
      click(rowOpenButton);
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    assert.equal(openDmCalls, 1, "a pending row suppresses a duplicate reopen");

    await act(async () => {
      release();
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    await inbox.settle();

    assert.equal(openDmCalls, 1, "still exactly one open_dm after it resolves");
    assert.deepEqual(inbox.navigations, [
      [HIDDEN_DM_ID, SOURCE_EVENT_ID, null],
    ]);
  } finally {
    await inbox.unmount();
  }
});

test("failed reopen surfaces a keyboard-operable Retry that then navigates", async () => {
  const inbox = await mountInbox();
  try {
    let attempts = 0;
    openDmHandler = async () => {
      attempts += 1;
      if (attempts === 1) throw new Error("relay offline");
      return { id: HIDDEN_DM_ID, channel_type: "dm" };
    };
    const openButton = inbox.container.querySelector(
      '[data-testid="home-inbox-item-' +
        SOURCE_EVENT_ID +
        '"] [aria-label="Open in channel"]',
    );
    await act(async () => {
      click(openButton);
    });
    await inbox.settle();

    assert.equal(attempts, 1, "one reopen attempt was made");
    assert.equal(
      inbox.navigations.length,
      0,
      "a failed reopen does not navigate",
    );
    const status = inbox.container.querySelector(
      '[data-testid="home-inbox-reopen-status"]',
    );
    assert.ok(status, "the error state must render in the status region");
    assert.match(status.textContent, /reopen/i);

    const retry = inbox.container.querySelector(
      '[data-testid="home-inbox-reopen-retry"]',
    );
    assert.ok(retry, "an explicit Retry control must render on failure");
    // Jude's regression was an unreachable affordance (a tooltip hidden behind
    // `pointer-events-none`). Prove the opposite: Retry is a native <button>
    // that is genuinely keyboard-reachable — focusable, enabled, and not
    // pointer-events-blocked — then activate it with a real keyboard event.
    assert.equal(retry.tagName, "BUTTON");
    assert.equal(retry.disabled, false);
    assert.notEqual(
      dom.window.getComputedStyle(retry).pointerEvents,
      "none",
      "Retry must not be pointer-events-blocked (Jude's regression)",
    );
    await act(async () => {
      retry.focus();
    });
    assert.equal(
      dom.window.document.activeElement,
      retry,
      "Retry must accept keyboard focus",
    );

    await act(async () => {
      // Drive the retry purely through the keyboard: an Enter keydown on the
      // focused native button triggers its default activation (installed in the
      // harness), which is what issues the retry command and navigation. No
      // synthetic click — the keyboard path alone must carry it.
      dom.window.document.activeElement.dispatchEvent(
        new dom.window.KeyboardEvent("keydown", {
          key: "Enter",
          bubbles: true,
          cancelable: true,
        }),
      );
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    await inbox.settle();

    assert.equal(attempts, 2, "Retry issues exactly one more reopen");
    assert.equal(openDmCalls, 2);
    assert.deepEqual(
      inbox.navigations,
      [[HIDDEN_DM_ID, SOURCE_EVENT_ID, null]],
      "the successful retry finally navigates",
    );
  } finally {
    await inbox.unmount();
  }
});

test("a failed reopen from an unselected row exposes its own keyboard Retry that navigates", async () => {
  // The selected detail is a DIFFERENT, already-open conversation, so the
  // detail pane's Retry belongs to OTHER_CHANNEL_ID and cannot reopen the
  // hidden DM. The failed hidden-DM row must therefore carry its own Retry.
  const inbox = await mountInbox({
    items: [OTHER_ITEM, HIDDEN_DM_ITEM],
    selectedItem: OTHER_ITEM,
  });
  try {
    let attempts = 0;
    openDmHandler = async () => {
      attempts += 1;
      if (attempts === 1) throw new Error("relay offline");
      return { id: HIDDEN_DM_ID, channel_type: "dm" };
    };
    const openButton = inbox.container.querySelector(
      '[data-testid="home-inbox-item-' +
        SOURCE_EVENT_ID +
        '"] [aria-label="Open in channel"]',
    );
    assert.ok(
      openButton,
      "the unselected hidden-DM row must render its action",
    );
    await act(async () => {
      click(openButton);
    });
    await inbox.settle();

    assert.equal(attempts, 1, "one reopen attempt was made");
    assert.equal(
      inbox.navigations.length,
      0,
      "a failed reopen does not navigate",
    );
    // The selected detail pane belongs to another channel, so its status
    // region must not be showing this failure.
    const detailStatus = inbox.container.querySelector(
      '[data-testid="home-inbox-reopen-status"]',
    );
    assert.equal(
      detailStatus,
      null,
      "the other-channel detail pane must not show the hidden DM's error",
    );

    // The failed row itself surfaces a keyboard-operable Retry.
    const rowStatus = inbox.container.querySelector(
      '[data-testid="home-inbox-reopen-status-' + SOURCE_EVENT_ID + '"]',
    );
    assert.ok(rowStatus, "the failed row must show its own error status");
    const retry = inbox.container.querySelector(
      '[data-testid="home-inbox-reopen-retry-' + SOURCE_EVENT_ID + '"]',
    );
    assert.ok(retry, "the failed row must expose its own Retry");
    assert.equal(retry.tagName, "BUTTON");
    assert.equal(retry.disabled, false);
    assert.notEqual(
      dom.window.getComputedStyle(retry).pointerEvents,
      "none",
      "the row Retry must not be pointer-events-blocked",
    );
    await act(async () => {
      retry.focus();
    });
    assert.equal(
      dom.window.document.activeElement,
      retry,
      "the row Retry must accept keyboard focus",
    );
    await act(async () => {
      dom.window.document.activeElement.dispatchEvent(
        new dom.window.KeyboardEvent("keydown", {
          key: "Enter",
          bubbles: true,
          cancelable: true,
        }),
      );
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
    await inbox.settle();

    assert.equal(attempts, 2, "the row Retry issues exactly one more reopen");
    assert.deepEqual(
      inbox.navigations,
      [[HIDDEN_DM_ID, SOURCE_EVENT_ID, null]],
      "the successful row retry navigates to the hidden DM",
    );
  } finally {
    await inbox.unmount();
  }
});
