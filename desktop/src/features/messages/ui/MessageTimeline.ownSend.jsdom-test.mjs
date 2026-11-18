/**
 * The user's own send reaches the bottom of the timeline even when Virtua
 * reports a gap from the floor around it.
 *
 * MessageTimeline freezes its live tail when Virtua reports that the reader
 * has left the bottom, and a send arms a one-shot pin for the next append. If
 * the freeze lands before the sent row reaches the deferred snapshot, the row
 * waits behind a "1 new message" pill, and the pin never sees its append.
 * Virtua can report that gap while the reader is at the floor: it counts rows
 * in its store during render, so a deferred render that React discards leaves
 * the sent row counted. The next scroll event, from the composer shrinking
 * after the send, then reads that row's height as distance from the bottom.
 */
import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  RouterContextProvider,
  createMemoryHistory,
  createRootRoute,
  createRouter,
} from "@tanstack/react-router";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import { MessageTimeline } from "./MessageTimeline.tsx";

const ROW = 60;
const VIEWPORT = 400;

// jsdom has no layout. Model what the timeline reads: the scroller's offset
// is clamped to the height Virtua gives its inner container, and the list
// host is as tall as the scroller so Virtua keeps an offscreen buffer.
const offsets = new WeakMap();
const isScroller = (element) => element?.style?.overflowY === "auto";
const maxOffset = (element) =>
  Math.max(
    (Number.parseFloat(element.firstElementChild?.style.height) || 0) -
      VIEWPORT,
    0,
  );
const clampOffset = (element, value) =>
  Math.min(Math.max(value, 0), maxOffset(element));
Object.defineProperties(window.HTMLElement.prototype, {
  clientHeight: {
    configurable: true,
    get() {
      return isScroller(this) || isScroller(this.firstElementChild)
        ? VIEWPORT
        : 0;
    },
  },
  scrollHeight: {
    configurable: true,
    get() {
      return isScroller(this) ? maxOffset(this) + VIEWPORT : 0;
    },
  },
  scrollTop: {
    configurable: true,
    get() {
      const offset = clampOffset(this, offsets.get(this) ?? 0);
      offsets.set(this, offset);
      return offset;
    },
    set(value) {
      offsets.set(this, clampOffset(this, value));
    },
  },
  offsetParent: {
    configurable: true,
    get() {
      return this.parentElement;
    },
  },
});

const resizeObservers = new Set();
globalThis.ResizeObserver = window.ResizeObserver = class {
  constructor(callback) {
    this.callback = callback;
    this.targets = new Set();
    resizeObservers.add(this);
  }
  observe(target) {
    this.targets.add(target);
  }
  unobserve(target) {
    this.targets.delete(target);
  }
  disconnect() {
    resizeObservers.delete(this);
  }
};

// Frames run only when the test flushes them, so it decides whether a queued
// freeze lands before or after the send.
let frames = new Map();
let nextFrameId = 1;
globalThis.requestAnimationFrame = window.requestAnimationFrame = (
  callback,
) => {
  const id = nextFrameId++;
  frames.set(id, callback);
  return id;
};
globalThis.cancelAnimationFrame = window.cancelAnimationFrame = (id) => {
  frames.delete(id);
};
function flushFrames() {
  act(() => {
    const pending = frames;
    frames = new Map();
    for (const callback of pending.values()) callback(performance.now());
  });
}

function message(index, body = `message ${index}`) {
  return {
    id: index.toString(16).padStart(64, "0"),
    createdAt: 1_700_000_000 + index,
    pubkey: "a".repeat(64),
    author: "alice",
    time: "1:00 AM",
    body,
    depth: 0,
    kind: 9,
    tags: [],
  };
}

const history = Array.from({ length: 20 }, (_, index) => message(index + 1));
const sent = message(21, "my own send");

let root;
let container;
afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
});

async function mountAtFloor(initial = history) {
  const router = createRouter({
    routeTree: createRootRoute(),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { enabled: false } },
  });
  const timeline = React.createRef();
  const render = (messages) =>
    act(async () =>
      root.render(
        React.createElement(
          RouterContextProvider,
          { router },
          React.createElement(
            QueryClientProvider,
            { client: queryClient },
            React.createElement(MessageTimeline, {
              channelId: "channel",
              messages,
              ref: timeline,
            }),
          ),
        ),
      ),
    );

  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
  await render(initial);
  const scroller = container.querySelector('[data-testid="message-timeline"]');
  act(() => {
    for (const observer of resizeObservers) {
      observer.callback(
        [...observer.targets].map((target) => ({
          target,
          contentRect: {
            width: 600,
            height: target === scroller ? VIEWPORT : ROW,
          },
        })),
      );
    }
  });
  flushFrames();
  readerScrollsTo(scroller, scroller.scrollHeight);
  flushFrames();
  return { render, scroller, timeline };
}

function readerScrollsTo(scroller, offset) {
  act(() => {
    scroller.scrollTop = offset;
    scroller.dispatchEvent(new window.Event("scroll"));
  });
}

// The guard cannot tell Virtua's stale gap from a real one, so a real 200px
// gap stands in for it. Either way, the send pulls the view to its row.
for (const { name, order } of [
  { name: "after the send", order: ["send", "gap"] },
  { name: "with its freeze still queued at the send", order: ["gap", "send"] },
]) {
  test(`an own send reaches the floor when Virtua reports a gap ${name}`, async () => {
    const { render, scroller, timeline } = await mountAtFloor();
    assert.equal(
      container.querySelector('[data-testid="message-scroll-to-latest"]'),
      null,
    );

    for (const step of order) {
      if (step === "send") {
        act(() => timeline.current.scrollToBottomOnNextUpdate("channel"));
      } else {
        readerScrollsTo(scroller, scroller.scrollTop - 200);
      }
    }
    flushFrames();
    await render([...history, sent]);
    flushFrames();

    assert.ok(container.querySelector(`[data-message-id="${sent.id}"]`));
    assert.equal(
      container.querySelector('[data-testid="message-scroll-to-latest"]'),
      null,
    );
    assert.equal(scroller.scrollTop, scroller.scrollHeight - VIEWPORT);
  });
}

// Rows arrive sorted by created_at, so a tail from a sender whose clock runs
// ahead stays the tail when the send lands above it, and the pin stays armed.
// The send's row still ends its hold on freezing: a reader who then scrolls up
// gets the next arrival behind the pill, even one that becomes the tail.
test("a reader who scrolls up after a send that lands above the tail keeps their place", async () => {
  const earlier = history.slice(0, -1);
  const aheadTail = { ...history.at(-1), createdAt: sent.createdAt + 3600 };
  const aheadNext = { ...message(22), createdAt: aheadTail.createdAt + 1 };
  const { render, scroller, timeline } = await mountAtFloor([
    ...earlier,
    aheadTail,
  ]);

  act(() => timeline.current.scrollToBottomOnNextUpdate("channel"));
  await render([...earlier, sent, aheadTail]);
  flushFrames();
  assert.ok(container.querySelector(`[data-message-id="${sent.id}"]`));

  readerScrollsTo(scroller, scroller.scrollTop - 200);
  const readingOffset = scroller.scrollTop;
  flushFrames();
  await render([...earlier, sent, aheadTail, aheadNext]);
  flushFrames();

  assert.equal(
    container.querySelector('[data-testid="message-scroll-to-latest"]')
      ?.textContent,
    "1 new message",
  );
  assert.equal(scroller.scrollTop, readingOffset);
});

// A failed send can leave no row here, as when its optimistic row never
// resolved a channel. Its cancel drops the pin and the hold, and freezes the
// tail the hold left open if the reader is off the floor, so the next arrival
// waits behind the pill instead of firing the pin under the reader.
for (const order of [
  ["send", "fail", "gap"],
  ["send", "gap", "fail"],
  ["gap", "send", "fail"],
]) {
  test(`a send that fails without a row leaves the reader in place (${order.join(", ")})`, async () => {
    const { render, scroller, timeline } = await mountAtFloor();

    for (const step of order) {
      if (step === "send") {
        act(() => timeline.current.scrollToBottomOnNextUpdate("channel"));
      } else if (step === "fail") {
        act(() => timeline.current.cancelScrollToBottomOnNextUpdate());
      } else {
        readerScrollsTo(scroller, scroller.scrollTop - 200);
      }
    }
    const readingOffset = scroller.scrollTop;
    flushFrames();
    await render([...history, message(22, "from the other person")]);
    flushFrames();

    assert.equal(
      container.querySelector('[data-testid="message-scroll-to-latest"]')
        ?.textContent,
      "1 new message",
    );
    assert.equal(scroller.scrollTop, readingOffset);
  });
}

// With the reader still at the floor, the hold skipped nothing, so the cancel
// leaves the tail open and the next arrival renders at once. A freeze here
// would only be released a frame later, holding that arrival behind the pill
// until then.
test("a send that fails at the floor keeps the timeline following", async () => {
  const { render, timeline } = await mountAtFloor();
  const arrival = message(22, "from the other person");

  act(() => timeline.current.scrollToBottomOnNextUpdate("channel"));
  act(() => timeline.current.cancelScrollToBottomOnNextUpdate());
  flushFrames();
  await render([...history, arrival]);

  assert.ok(container.querySelector(`[data-message-id="${arrival.id}"]`));
  assert.equal(
    container.querySelector('[data-testid="message-scroll-to-latest"]'),
    null,
  );
});

// A send to this channel lands its optimistic row at once, which is an append
// that fires the pin and ends the hold. When the publish then fails, the row
// is reverted and the cancel finds nothing pending: it must not re-arm the
// freeze's bottom-report suppression, or returning to the floor would leave
// the arrival stuck behind the pill.
test("a reader who scrolls up before a same-channel send fails keeps their place", async () => {
  const { render, scroller, timeline } = await mountAtFloor();

  act(() => timeline.current.scrollToBottomOnNextUpdate("channel"));
  await render([...history, sent]);
  flushFrames();
  assert.equal(scroller.scrollTop, scroller.scrollHeight - VIEWPORT);

  readerScrollsTo(scroller, scroller.scrollTop - 200);
  const readingOffset = scroller.scrollTop;
  flushFrames();
  act(() => timeline.current.cancelScrollToBottomOnNextUpdate());
  await render(history);
  flushFrames();
  const arrival = message(22, "from the other person");
  await render([...history, arrival]);
  flushFrames();

  assert.equal(
    container.querySelector('[data-testid="message-scroll-to-latest"]')
      ?.textContent,
    "1 new message",
  );
  assert.equal(scroller.scrollTop, readingOffset);

  readerScrollsTo(scroller, scroller.scrollHeight);
  flushFrames();
  assert.ok(container.querySelector(`[data-message-id="${arrival.id}"]`));
  assert.equal(
    container.querySelector('[data-testid="message-scroll-to-latest"]'),
    null,
  );
});

// A send whose upload spans a channel switch settles with the channel it was
// written in. Its arm must not release this timeline's frozen tail or pin its
// next arrival under a reader who scrolled up.
test("an own send for another channel leaves this timeline's reader in place", async () => {
  const { render, scroller, timeline } = await mountAtFloor();
  readerScrollsTo(scroller, scroller.scrollTop - 200);
  const readingOffset = scroller.scrollTop;
  flushFrames();
  await render([...history, message(22, "from the other person")]);
  flushFrames();

  act(() => timeline.current.scrollToBottomOnNextUpdate("design"));
  flushFrames();
  await render([
    ...history,
    message(22, "from the other person"),
    message(23, "and again"),
  ]);
  flushFrames();

  assert.equal(
    container.querySelector('[data-testid="message-scroll-to-latest"]')
      ?.textContent,
    "2 new messages",
  );
  assert.equal(scroller.scrollTop, readingOffset);
});
