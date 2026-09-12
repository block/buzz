import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

import {
  isWindowDragHandleEvent,
  markAllReadSources,
  activateDesktopNotificationTarget,
  createDesktopNotificationActivationQueue,
  shouldBounceForChannelNotification,
} from "./AppShell.helpers.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    Element: dom.window.Element,
    HTMLElement: dom.window.HTMLElement,
    document: dom.window.document,
    window: dom.window,
  });
});

after(() => dom.window.close());

// `composedPath()` is only populated during dispatch, so the helper has to be
// called from a live listener rather than on a synthesized event object.
function dragHandleVerdictAt(element, clientY) {
  let verdict;
  const listener = (event) => {
    verdict = isWindowDragHandleEvent(event);
  };
  dom.window.addEventListener("pointerdown", listener, true);
  element.dispatchEvent(
    new dom.window.MouseEvent("pointerdown", { bubbles: true, clientY }),
  );
  dom.window.removeEventListener("pointerdown", listener, true);
  return verdict;
}

test("window drag fallback leaves content that sits under the top strip selectable", () => {
  // Regression for #6827: the fallback was purely geometric, so any click in
  // the top 44px — including message text scrolled up there — was swallowed as
  // a title-bar drag and could never start a selection.
  dom.window.document.body.innerHTML = `
    <div data-tauri-drag-region><span id="chrome-label">Buzz</span></div>
    <main><p id="message">selectable message text</p></main>
  `;
  const message = dom.window.document.getElementById("message");
  assert.equal(dragHandleVerdictAt(message, 42), false);
});

test("window drag fallback still covers plain children of a drag region", () => {
  dom.window.document.body.innerHTML = `
    <div data-tauri-drag-region><span id="chrome-label">Buzz</span></div>
    <main><p id="message">selectable message text</p></main>
  `;
  const label = dom.window.document.getElementById("chrome-label");
  assert.equal(dragHandleVerdictAt(label, 42), true);
});

test("shouldBounceForChannelNotification_allowsTopLevelChannelMessages", () => {
  assert.equal(shouldBounceForChannelNotification([["h", "channel"]]), true);
});

test("shouldBounceForChannelNotification_suppressesThreadReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
    ]),
    false,
  );
});

test("shouldBounceForChannelNotification_allowsBroadcastReplies", () => {
  assert.equal(
    shouldBounceForChannelNotification([
      ["h", "channel"],
      ["e", "root", "", "reply"],
      ["broadcast", "1"],
    ]),
    true,
  );
});

test("notification activation queue preserves click order", async () => {
  const calls = [];
  const resolvers = new Map();
  const queue = createDesktopNotificationActivationQueue((target) => {
    calls.push(`start:${target.channelId}`);
    return new Promise((resolve) => {
      resolvers.set(target.channelId, () => {
        calls.push(`finish:${target.channelId}`);
        resolve();
      });
    });
  });

  queue.enqueue({ channelId: "first", eventId: null, kind: null });
  queue.enqueue({ channelId: "second", eventId: null, kind: null });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["start:first"]);

  resolvers.get("first")();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["start:first", "finish:first", "start:second"]);

  resolvers.get("second")();
});

test("notification activation queue drops pending targets after cancellation", async () => {
  const calls = [];
  let resolveFirst;
  const queue = createDesktopNotificationActivationQueue((target) => {
    calls.push(target.channelId);
    if (target.channelId === "first") {
      return new Promise((resolve) => {
        resolveFirst = resolve;
      });
    }
    return Promise.resolve();
  });

  queue.enqueue({ channelId: "first", eventId: null, kind: null });
  queue.enqueue({ channelId: "second", eventId: null, kind: null });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["first"]);

  queue.cancel();
  resolveFirst();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["first"]);
});

test("notification activation queue aborts an in-flight activation", async () => {
  let observedSignal;
  let resolveActivation;
  const queue = createDesktopNotificationActivationQueue((_target, signal) => {
    observedSignal = signal;
    return new Promise((resolve) => {
      resolveActivation = resolve;
    });
  });

  queue.enqueue({ channelId: "first", eventId: null, kind: null });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(observedSignal.aborted, false);

  queue.cancel();
  assert.equal(observedSignal.aborted, true);
  resolveActivation();
});

test("notification activation queue reports failures and continues", async () => {
  const calls = [];
  const errors = [];
  const queue = createDesktopNotificationActivationQueue(
    async (target) => {
      calls.push(target.channelId);
      if (target.channelId === "first") {
        throw new Error("navigation failed");
      }
    },
    (error) => errors.push(error),
  );

  queue.enqueue({ channelId: "first", eventId: null, kind: null });
  queue.enqueue({ channelId: "second", eventId: null, kind: null });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(calls, ["first", "second"]);
  assert.equal(errors.length, 1);
  assert.match(errors[0].message, /navigation failed/);
});

test("notification activation starts routing before a hung reveal", async () => {
  const calls = [];
  let resolveNavigation;
  let navigationSettled = false;
  const activation = activateDesktopNotificationTarget(
    {
      channelId: "channel",
      eventId: "event",
      kind: 9,
    },
    {
      goChannel: async () => calls.push("channel"),
      goHome: async () => calls.push("home"),
      revealWindow: () => new Promise(() => {}),
      openSearchHit: (_hit, behavior) => {
        calls.push(`message:${String(behavior?.force)}`);
        return new Promise((resolve) => {
          resolveNavigation = () => {
            navigationSettled = true;
            resolve();
          };
        });
      },
    },
  );

  assert.deepEqual(calls, ["message:true"]);
  resolveNavigation();
  await activation;
  assert.equal(navigationSettled, true);
});

test("notification activation falls back to forced channel navigation", async () => {
  const calls = [];
  await activateDesktopNotificationTarget(
    { channelId: "channel", eventId: null, kind: null },
    {
      goChannel: async (channelId, behavior) =>
        calls.push(`${channelId}:${String(behavior?.force)}`),
      goHome: async () => calls.push("home"),
      openSearchHit: async () => calls.push("message"),
      revealWindow: async () => calls.push("reveal"),
    },
  );

  assert.deepEqual(calls, ["channel:true", "reveal"]);
});

test("notification activation ignores reveal rejection after routing starts", async () => {
  const calls = [];
  await activateDesktopNotificationTarget(
    { channelId: "channel", eventId: null, kind: null },
    {
      goChannel: async () => calls.push("channel"),
      goHome: async () => calls.push("home"),
      openSearchHit: async () => calls.push("message"),
      revealWindow: async () => {
        calls.push("reveal");
        throw new Error("reveal failed");
      },
    },
  );

  assert.deepEqual(calls, ["channel", "reveal"]);
});

test("notification activation without a channel opens home", async () => {
  const calls = [];
  await activateDesktopNotificationTarget(
    { channelId: null, eventId: "event", kind: 9 },
    {
      goChannel: async () => calls.push("channel"),
      goHome: async () => calls.push("home"),
      openSearchHit: async () => calls.push("message"),
      revealWindow: async () => calls.push("reveal"),
    },
  );

  assert.deepEqual(calls, ["home", "reveal"]);
});

test("markAllReadSources clears Inbox overrides and active thread activity", () => {
  const calls = [];

  markAllReadSources({
    activeChannelId: "active-channel",
    channelActivityItems: [
      { channelId: "another-channel", createdAt: 100 },
      { channelId: "active-channel", createdAt: 200 },
      { channelId: "active-channel", createdAt: 300 },
    ],
    unreadFeedItemIds: new Set(["first-inbox-item", "second-inbox-item"]),
    undoUnreadFeedItem: (itemId) => calls.push(`inbox:${itemId}`),
    markAllChannelReadMarkers: () => calls.push("channels"),
    markActiveChannelRead: (channelId, createdAt) =>
      calls.push(`active:${channelId}:${createdAt}`),
  });

  assert.deepEqual(calls, [
    "inbox:first-inbox-item",
    "inbox:second-inbox-item",
    "channels",
    "active:active-channel:300",
  ]);
});

test("markAllReadSources skips the active marker without projected activity", () => {
  const calls = [];

  markAllReadSources({
    activeChannelId: "active-channel",
    channelActivityItems: [],
    unreadFeedItemIds: new Set(),
    undoUnreadFeedItem: () => calls.push("inbox"),
    markAllChannelReadMarkers: () => calls.push("channels"),
    markActiveChannelRead: () => calls.push("active"),
  });

  assert.deepEqual(calls, ["channels"]);
});
