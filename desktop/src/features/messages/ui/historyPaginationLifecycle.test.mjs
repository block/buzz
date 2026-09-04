import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { useUpwardPaginationWheel } from "./useUpwardPaginationWheel.ts";
import { useSettleGatedPrependMessages } from "./useSettleGatedPrependMessages.ts";

async function setup(t) {
  const dom = new JSDOM(
    "<div id='root'></div><div id='host'><div></div></div>",
  );
  const frames = new Map();
  let frameId = 0;
  let now = 0;
  const globals = {
    window: dom.window,
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    HTMLDivElement: dom.window.HTMLDivElement,
    Node: dom.window.Node,
    IS_REACT_ACT_ENVIRONMENT: true,
    requestAnimationFrame(fn) {
      frames.set(++frameId, fn);
      return frameId;
    },
    cancelAnimationFrame(id) {
      frames.delete(id);
    },
  };
  const saved = Object.fromEntries(
    Object.keys(globals).map((key) => [
      key,
      Object.getOwnPropertyDescriptor(globalThis, key),
    ]),
  );
  for (const [key, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, key, {
      value,
      configurable: true,
      writable: true,
    });
  }
  const originalNow = Object.getOwnPropertyDescriptor(performance, "now");
  Object.defineProperty(performance, "now", {
    value: () => now,
    configurable: true,
  });
  const host = document.getElementById("host");
  const scroller = host.firstElementChild;
  Object.defineProperties(scroller, {
    scrollHeight: { value: 3000 },
    clientHeight: { value: 600 },
  });
  const root = createRoot(document.getElementById("root"));
  t.after(async () => {
    await act(async () => {
      root.unmount();
      // Query's notifyManager batches mutation-observer callbacks on a timer.
      // Await delivery while JSDOM still exists, even after the cache mutation
      // promise has settled; those callbacks can still enter React DOM.
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    dom.window.close();
    if (originalNow) Object.defineProperty(performance, "now", originalNow);
    else delete performance.now;
    for (const [key, descriptor] of Object.entries(saved)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  });
  return {
    root,
    scroller,
    hostRef: { current: host },
    async frame(ms = 16) {
      now += ms;
      const pending = [...frames.values()];
      frames.clear();
      await act(async () => {
        for (const fn of pending) fn(now);
      });
    },
    wheel(deltaY = -10) {
      const event = new dom.window.WheelEvent("wheel", {
        deltaY,
        cancelable: true,
      });
      scroller.dispatchEvent(event);
      return event;
    },
  };
}

test("last-tick paging cannot swallow a new wheel gesture after a pause", async (t) => {
  const env = await setup(t);
  let paging;
  const onWheel = () => {};
  function Harness() {
    paging = useUpwardPaginationWheel(env.hostRef, onWheel);
    return null;
  }
  await act(async () => env.root.render(React.createElement(Harness)));
  env.wheel();
  paging.arm(true);
  await env.frame(1000);
  assert.equal(env.wheel().defaultPrevented, false);
});

test("fresh input past the old hold deadline never admits a prepend", async (t) => {
  const env = await setup(t);
  const scrollElementRef = { current: env.scroller };
  let output;
  function Harness({ messages }) {
    output = useSettleGatedPrependMessages({
      channelId: "a",
      messages,
      meta: messages[0].id,
      scrollElementRef,
    });
    return null;
  }
  const oldRows = [{ id: "a" }, { id: "b" }];
  const nextRows = [{ id: "older" }, ...oldRows];
  await act(async () =>
    env.root.render(React.createElement(Harness, { messages: oldRows })),
  );
  await act(async () =>
    env.root.render(React.createElement(Harness, { messages: nextRows })),
  );
  assert.equal(output.isHoldingPrepend, true);
  for (let i = 0; i < 85; i++) {
    env.wheel();
    await env.frame(50);
  }
  assert.equal(
    output.isHoldingPrepend,
    true,
    "4 seconds is not proof that a new gesture has settled",
  );
  assert.deepEqual(output.messages, oldRows);
  for (let i = 0; i < 8; i++) await env.frame();
  assert.equal(output.isHoldingPrepend, false);
  assert.deepEqual(output.messages, nextRows);
  assert.equal(output.meta, "older");
});

// Exercise the production transport and admission hooks, not a copied selector.
const { useAdmittedTimelineSnapshot } = await import(
  "./useAdmittedTimelineSnapshot.ts"
);
const { useHistoryPagination } = await import("./useHistoryPagination.ts");

test("page reservation spans fast fetch, React deferral, settle hold and visual acknowledgement", async (t) => {
  const env = await setup(t);
  const scrollElementRef = { current: env.scroller };
  let publish;
  let pager;
  let rows;
  let requests = 0;
  let probeNextUrgentCommit = false;
  const admissionDuringGap = [];
  const oldRows = [{ id: "a" }, { id: "b" }];
  const olderRows = [{ id: "older" }, ...oldRows];
  const initial = {
    channelId: "a",
    messages: oldRows,
    historyExhausted: false,
    historyRevision: 1,
    firstUnreadMessageId: "a",
  };
  function Harness() {
    const [snapshot, setSnapshot] = React.useState(initial);
    publish = setSnapshot;
    const { admitted } = useAdmittedTimelineSnapshot({
      snapshot,
      isAtBottom: snapshot.historyRevision === 1,
      scrollElementRef,
    });
    rows = admitted;
    pager = useHistoryPagination({
      channelId: snapshot.channelId,
      canLoad: true,
      renderedRevision: admitted.meta.historyRevision,
      scrollElementRef,
      fetchOlder: async () => {
        requests++;
        return 2;
      },
    });
    React.useLayoutEffect(() => {
      if (
        probeNextUrgentCommit &&
        snapshot.historyRevision === 2 &&
        admitted.meta.historyRevision === 1
      ) {
        probeNextUrgentCommit = false;
        admissionDuringGap.push(pager.start());
      }
    });
    return null;
  }
  await act(async () => env.root.render(React.createElement(Harness)));
  await act(async () => {
    assert.equal(pager.start(), true);
    assert.equal(
      pager.start(),
      false,
      "sync second callback cannot reserve a second request",
    );
  });
  assert.equal(
    pager.isPending,
    true,
    "network receipt alone does not release reservation",
  );
  probeNextUrgentCommit = true;
  await act(async () =>
    publish({
      ...initial,
      messages: olderRows,
      historyRevision: 2,
      historyExhausted: true,
      firstUnreadMessageId: null,
    }),
  );
  assert.deepEqual(
    admissionDuringGap,
    [false],
    "production deferral urgent commit stays locked",
  );
  assert.equal(rows.isHoldingPrepend, true);
  assert.equal(
    rows.meta.firstUnreadMessageId,
    "a",
    "marker belongs to held rows",
  );
  for (let i = 0; i < 8; i++) await env.frame();
  assert.deepEqual(rows.messages, olderRows);
  assert.equal(rows.meta.firstUnreadMessageId, null);
  assert.equal(
    pager.isPending,
    true,
    "DOM commit still awaits stable measurements",
  );
  for (let i = 0; i < 5; i++) await env.frame();
  assert.equal(pager.isPending, false);
  assert.equal(requests, 1);
});

test("navigation retires old request receipt without blocking the new channel", async (t) => {
  const env = await setup(t);
  const scrollElementRef = { current: env.scroller };
  let pager;
  const resolves = [];
  const fetchOlder = () => new Promise((resolve) => resolves.push(resolve));
  function Harness({ channelId }) {
    pager = useHistoryPagination({
      channelId,
      fetchOlder,
      canLoad: true,
      renderedRevision: 1,
      scrollElementRef,
    });
    return null;
  }
  await act(async () =>
    env.root.render(React.createElement(Harness, { channelId: "a" })),
  );
  await act(async () => assert.equal(pager.start(), true));
  await act(async () =>
    env.root.render(React.createElement(Harness, { channelId: "b" })),
  );
  assert.equal(pager.isPending, false);
  await act(async () => assert.equal(pager.start(), true));
  await act(async () => resolves[0](50));
  assert.equal(
    pager.isPending,
    true,
    "old channel cannot unlock the active request",
  );
  await act(async () => resolves[1](undefined));
  assert.equal(
    pager.isPending,
    false,
    "no-op receipt releases the active request",
  );
});

const { useHistoryBoundaryIntent } = await import(
  "./useHistoryBoundaryIntent.ts"
);

test("only fresh reader intent pages once; layout scrolls cannot create or reuse a gesture", async (t) => {
  const env = await setup(t);
  let starts = 0;
  let canStart = true;
  let tryStart;
  function Harness() {
    tryStart = useHistoryBoundaryIntent(
      env.hostRef,
      () => {
        if (!canStart) return false;
        starts++;
        return true;
      },
      () => {},
    );
    return null;
  }
  await act(async () => env.root.render(React.createElement(Harness)));
  tryStart();
  assert.equal(starts, 0, "programmatic offset alone is not input");
  env.wheel();
  await env.frame();
  assert.equal(starts, 1, "upward wheel at zero still starts a page");
  for (let i = 0; i < 30; i++) {
    env.wheel();
    await env.frame(40);
    tryStart();
  }
  assert.equal(
    starts,
    1,
    "a long continuous gesture cannot cascade even after receipt settles",
  );
  await env.frame(200);
  env.wheel();
  await env.frame();
  assert.equal(starts, 2, "new gesture can load next page");
  canStart = false;
  await env.frame(200);
  env.wheel();
  await env.frame();
  canStart = true;
  await env.frame(200);
  tryStart();
  assert.equal(
    starts,
    2,
    "rejected input cannot be reused by later layout motion",
  );
  const key = new window.KeyboardEvent("keydown", { key: "PageUp" });
  env.scroller.dispatchEvent(key);
  await env.frame();
  assert.equal(starts, 3, "keyboard history navigation supplies intent");
  await env.frame(200);
  env.scroller.dispatchEvent(
    new window.WheelEvent("wheel", { deltaY: -10, ctrlKey: true }),
  );
  await env.frame();
  assert.equal(starts, 3, "zoom cannot page");
});

test("returning to a channel cannot resurrect its retired transaction indicator", async (t) => {
  const env = await setup(t);
  const scrollElementRef = { current: env.scroller };
  let pager;
  let resolve;
  function Harness({ channelId }) {
    pager = useHistoryPagination({
      channelId,
      fetchOlder: () =>
        new Promise((done) => {
          resolve = done;
        }),
      canLoad: true,
      renderedRevision: 1,
      scrollElementRef,
    });
    return null;
  }
  await act(async () =>
    env.root.render(React.createElement(Harness, { channelId: "a" })),
  );
  await act(async () => pager.start());
  await act(async () =>
    env.root.render(React.createElement(Harness, { channelId: "b" })),
  );
  await act(async () =>
    env.root.render(React.createElement(Harness, { channelId: "a" })),
  );
  assert.equal(pager.isPending, false);
  await act(async () => resolve(2));
  assert.equal(pager.isPending, false);
});

// Real production projection hook, with intentionally separated cache/store
// notifications: the receipt, rows, summaries and exhaustion must still agree.
const { useHuddleChannelMessages } = await import(
  "../../channels/ui/useHuddleChannelMessages.ts"
);
const { QueryClient, QueryClientProvider } = await import(
  "@tanstack/react-query"
);
const {
  emptyChannelWindowStore,
  replaceNewestChannelWindow,
  appendOlderChannelWindow,
} = await import("../lib/channelWindowStore.ts");
test("production channel projection pairs a window receipt with its rows and structural metadata", async (t) => {
  const env = await setup(t);
  const client = new QueryClient();
  t.after(() => client.clear());
  const event = (id, created_at) => ({
    id,
    created_at,
    kind: 9,
    pubkey: "p",
    content: id,
    tags: [["h", "a"]],
    sig: "",
  });
  const headRow = event("newer", 2);
  const oldRow = event("older", 1);
  const summary = {
    replyCount: 2,
    descendantCount: 2,
    participantPubkeys: [],
    lastReplyAt: 1,
  };
  const cursor = { createdAt: 2, eventId: "newer" };
  const head = replaceNewestChannelWindow(emptyChannelWindowStore(), {
    startCursor: null,
    rows: [{ event: headRow, thread: null }],
    aux: [],
    nextCursor: cursor,
    hasMore: true,
  });
  const next = appendOlderChannelWindow(head, {
    startCursor: cursor,
    rows: [{ event: oldRow, thread: summary }],
    aux: [],
    nextCursor: null,
    hasMore: false,
  });
  let output;
  function Harness({ windowStore, messages }) {
    output = useHuddleChannelMessages({
      activeChannel: { id: "a", channelType: "stream" },
      isHuddleTranscript: false,
      windowStore,
      messages,
      targetMessageEvents: [],
    });
    return null;
  }
  const render = async (windowStore, messages) =>
    act(async () =>
      env.root.render(
        React.createElement(
          QueryClientProvider,
          { client },
          React.createElement(Harness, { windowStore, messages }),
        ),
      ),
    );
  await render(head, [headRow]);
  await render(next, [headRow]); // new window, old flattened-cache observer
  assert.deepEqual(
    output.resolvedMessages.map((row) => row.id),
    ["older", "newer"],
  );
  assert.equal(output.historyRevision, next.revision);
  assert.equal(output.historyExhausted, true);
  assert.equal(output.threadSummaries.get("older"), summary);
  await render(head, [oldRow, headRow]); // inverse observer order
  assert.deepEqual(
    output.resolvedMessages.map((row) => row.id),
    ["newer"],
  );
  assert.equal(output.historyRevision, head.revision);
  assert.equal(output.historyExhausted, false);
  assert.equal(output.threadSummaries.has("older"), false);
});

test("explicit cancellation and failed/no-op requests release only their own reservation", async (t) => {
  const env = await setup(t);
  const scrollElementRef = { current: env.scroller };
  const pending = [];
  const errors = [];
  t.mock.method(console, "error", (...args) => errors.push(args));
  let pager;
  function Harness({ canLoad = true, renderedRevision = 1 }) {
    pager = useHistoryPagination({
      channelId: "a",
      fetchOlder: () =>
        new Promise((resolve, reject) => pending.push({ resolve, reject })),
      canLoad,
      renderedRevision,
      scrollElementRef,
    });
    return null;
  }
  await act(async () =>
    env.root.render(React.createElement(Harness, { canLoad: false })),
  );
  assert.equal(
    pager.start(),
    false,
    "exhausted/navigation states do not request",
  );
  await act(async () => env.root.render(React.createElement(Harness)));
  await act(async () => pager.start());
  await act(async () => pager.cancel());
  assert.equal(pager.isPending, false);
  await act(async () => pager.start());
  await act(async () => pending[0].resolve(30));
  assert.equal(
    pager.isPending,
    true,
    "cancelled receipt cannot release a newer request",
  );
  await act(async () => pending[1].reject(Error("offline")));
  assert.equal(pager.isPending, false);
  assert.equal(errors.length, 1);
  await act(async () => pager.start());
  await act(async () => pending[2].resolve(undefined));
  assert.equal(pager.isPending, false);
  await act(async () => pager.start());
  await act(async () => pending[3].resolve(2));
  for (let i = 0; i < 8; i++) await env.frame();
  assert.equal(
    pager.isPending,
    true,
    "geometry alone cannot acknowledge an unrendered receipt",
  );
  await act(async () =>
    env.root.render(React.createElement(Harness, { renderedRevision: 2 })),
  );
  for (let i = 0; i < 3; i++) await env.frame();
  env.scroller.scrollTop += 20;
  await env.frame();
  assert.equal(
    pager.isPending,
    true,
    "late layout resets the stable-frame count",
  );
  for (let i = 0; i < 3; i++) await env.frame();
  assert.equal(pager.isPending, false);
});

test("keyboard input ignores editing/modifiers and consumes only one held key", async (t) => {
  const env = await setup(t);
  let starts = 0;
  function Harness() {
    useHistoryBoundaryIntent(
      env.hostRef,
      () => {
        starts++;
        return true;
      },
      () => {},
    );
    return null;
  }
  await act(async () => env.root.render(React.createElement(Harness)));
  const key = async (target, options) => {
    target.dispatchEvent(
      new window.KeyboardEvent("keydown", { bubbles: true, ...options }),
    );
    await env.frame();
  };
  const editor = document.createElement("div");
  editor.setAttribute("contenteditable", "true");
  const child = document.createElement("span");
  editor.append(child);
  env.scroller.append(editor);
  await key(child, { key: "Home" });
  await key(env.scroller, { key: "Home", metaKey: true });
  await key(env.scroller, { key: "PageDown" });
  assert.equal(starts, 0);
  await key(env.scroller, { key: "PageUp" });
  for (let i = 0; i < 8; i++)
    await key(env.scroller, { key: "PageUp", repeat: true });
  assert.equal(starts, 1);
  await key(env.scroller, { key: " ", shiftKey: true });
  assert.equal(starts, 2, "a distinct keypress can supply fresh upward intent");
});

test("touch intent requires upward motion and does not rearm during a held finger pause", async (t) => {
  const env = await setup(t);
  let starts = 0;
  let tryStart;
  function Harness() {
    tryStart = useHistoryBoundaryIntent(
      env.hostRef,
      () => {
        starts++;
        return true;
      },
      () => {},
    );
    return null;
  }
  await act(async () => env.root.render(React.createElement(Harness)));
  const touch = async (name, y) => {
    env.scroller.dispatchEvent(
      new window.TouchEvent(name, { touches: [{ clientY: y }] }),
    );
    await env.frame();
  };
  await touch("touchstart", 100);
  tryStart();
  assert.equal(starts, 0, "touch down alone is not history intent");
  await touch("touchmove", 80);
  assert.equal(starts, 0, "downward reading motion does not page");
  await touch("touchmove", 120);
  assert.equal(starts, 1);
  await env.frame(500);
  await touch("touchmove", 150);
  assert.equal(
    starts,
    1,
    "a held touch remains the same gesture across a pause",
  );
  await touch("touchend", 150);
  await touch("touchstart", 100);
  await touch("touchmove", 150);
  assert.equal(starts, 2);
  await touch("touchcancel", 150);
  tryStart();
  assert.equal(starts, 2);
});

test("scrollbar intent requires upward drag; click, cancellation and later layout cannot page", async (t) => {
  const env = await setup(t);
  let starts = 0;
  let tryStart;
  function Harness() {
    tryStart = useHistoryBoundaryIntent(
      env.hostRef,
      () => {
        starts++;
        return true;
      },
      () => {},
    );
    return null;
  }
  await act(async () => env.root.render(React.createElement(Harness)));
  const pointer = async (name, target = env.scroller) => {
    target.dispatchEvent(new window.Event(name, { bubbles: true }));
    await env.frame();
  };
  await pointer("pointerdown");
  assert.equal(starts, 0, "clicking unused scroller space is not upward input");
  env.scroller.scrollTop = 20;
  env.scroller.dispatchEvent(new window.Event("scroll"));
  assert.equal(starts, 0, "downward scrollbar travel does not page");
  env.scroller.scrollTop = 10;
  env.scroller.dispatchEvent(new window.Event("scroll"));
  assert.equal(starts, 1);
  await env.frame(500);
  env.scroller.scrollTop = 0;
  env.scroller.dispatchEvent(new window.Event("scroll"));
  assert.equal(starts, 1, "one drag cannot consume multiple pages");
  await pointer("pointercancel", window);
  await pointer("pointerdown");
  await pointer("pointercancel", window);
  env.scroller.scrollTop = 0;
  tryStart();
  assert.equal(starts, 1, "cancelled pointer cannot lend intent to layout");
});

test("empty viewport fill is serial, receipt-gated and capped at three pages", async (t) => {
  const env = await setup(t);
  const emptyScroller = document.createElement("div");
  Object.defineProperties(emptyScroller, {
    scrollHeight: { value: 600 },
    clientHeight: { value: 600 },
  });
  const scrollElementRef = { current: emptyScroller };
  let requests = 0;
  let pager;
  let revision = 1;
  function Harness({ renderedRevision }) {
    pager = useHistoryPagination({
      channelId: "a",
      canLoad: true,
      fillViewport: true,
      renderedRevision,
      scrollElementRef,
      fetchOlder: async () => {
        requests++;
        return revision + 1;
      },
    });
    return null;
  }
  const render = () =>
    act(async () =>
      env.root.render(
        React.createElement(Harness, { renderedRevision: revision }),
      ),
    );
  await render();
  for (let i = 0; i < 8; i++) await env.frame();
  assert.equal(requests, 1);
  for (let i = 0; i < 20; i++) await env.frame();
  assert.equal(requests, 1, "fill does not bypass visual acknowledgement");
  for (let step = 0; step < 3; step++) {
    revision++;
    await render();
    for (let i = 0; i < 10; i++) await env.frame();
  }
  assert.equal(requests, 3, "an unfillable viewport cannot loop indefinitely");
  assert.equal(pager.isPending, false);
  await act(async () => assert.equal(pager.start(), true));
  assert.equal(requests, 4, "the fill cap does not disable deliberate paging");
});

const { useSendMessageMutation } = await import("../hooks.ts");
const { relayClient } = await import("../../../shared/api/relayClient.ts");
const { channelWindowKey, channelMessagesKey } = await import(
  "../lib/messageQueryKeys.ts"
);
const { mergeLiveChannelWindowEvent } = await import(
  "../lib/channelWindowStore.ts"
);
const { projectChannelWindowMessages } = await import(
  "../lib/projectChannelWindow.ts"
);

test("failed send removes only its optimistic row, retaining concurrent history and live writes", async (t) => {
  const env = await setup(t);
  const client = new QueryClient({
    defaultOptions: {
      mutations: { retry: false, gcTime: Infinity },
      queries: { gcTime: Infinity },
    },
  });
  t.after(() => client.clear());
  let rejectSend;
  let sendStarted;
  const started = new Promise((resolve) => {
    sendStarted = resolve;
  });
  t.mock.method(relayClient, "sendMessage", () => {
    sendStarted();
    return new Promise((_, reject) => {
      rejectSend = reject;
    });
  });
  const event = (id, created_at) => ({
    id,
    created_at,
    kind: 9,
    pubkey: "p",
    content: id,
    tags: [["h", "a"]],
    sig: "",
  });
  const headRow = event("head", 100);
  const cursor = { eventId: headRow.id, createdAt: 100 };
  const head = replaceNewestChannelWindow(emptyChannelWindowStore(), {
    startCursor: null,
    rows: [{ event: headRow, thread: null }],
    aux: [],
    nextCursor: cursor,
    hasMore: true,
  });
  const key = channelWindowKey("a");
  client.setQueryData(key, head);
  projectChannelWindowMessages(client, "a");
  let mutation;
  function Harness() {
    mutation = useSendMessageMutation(
      { id: "a", channelType: "stream" },
      { pubkey: "p" },
    );
    return null;
  }
  await act(async () =>
    env.root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(Harness),
      ),
    ),
  );
  let result;
  await act(async () => {
    result = mutation
      .mutateAsync({ content: "will fail" })
      .catch((error) => error);
    await started;
  });
  const pendingId = client.getQueryData(key).liveOverlay[0].id;
  let current = appendOlderChannelWindow(client.getQueryData(key), {
    startCursor: cursor,
    rows: [{ event: event("older", 90), thread: null }],
    aux: [],
    nextCursor: null,
    hasMore: false,
  });
  current = mergeLiveChannelWindowEvent(current, event("live", 110));
  current = mergeLiveChannelWindowEvent(current, {
    ...event("other-pending", 120),
    pending: true,
  });
  client.setQueryData(key, current);
  projectChannelWindowMessages(client, "a");
  await act(async () => {
    rejectSend(Error("offline"));
    await result;
  });
  const after = client.getQueryData(key);
  assert.equal(
    after.revision,
    current.revision,
    "send rollback cannot regress a publication receipt",
  );
  assert.equal(after.pages.length, 2);
  assert.equal(
    after.liveOverlay.some((row) => row.id === pendingId),
    false,
  );
  assert.deepEqual(
    after.liveOverlay.map((row) => row.id),
    ["other-pending", "live"],
  );
  assert.deepEqual(
    client.getQueryData(channelMessagesKey("a")).map((row) => row.id),
    ["older", "head", "live", "other-pending"],
  );
});

const { useDeleteMessageMutation } = await import("../hooks.ts");

test("accepted deletion changes the authoritative window even without a live deletion echo", async (t) => {
  const env = await setup(t);
  const client = new QueryClient({
    defaultOptions: {
      queries: { gcTime: Infinity },
      mutations: { retry: false, gcTime: Infinity },
    },
  });
  t.after(() => client.clear());
  let acceptDelete;
  let deleteStarted;
  const started = new Promise((resolve) => {
    deleteStarted = resolve;
  });
  window.__TAURI_INTERNALS__ = {
    invoke: async (command) => {
      assert.equal(command, "delete_message");
      deleteStarted();
      await new Promise((resolve) => {
        acceptDelete = resolve;
      });
    },
  };
  const event = (id, created_at) => ({
    id,
    created_at,
    kind: 9,
    pubkey: "p",
    content: id,
    tags: [["h", "a"]],
    sig: "",
  });
  const target = event("target", 100);
  const key = channelWindowKey("a");
  client.setQueryData(
    key,
    replaceNewestChannelWindow(emptyChannelWindowStore(), {
      startCursor: null,
      rows: [{ event: target, thread: null }],
      aux: [],
      nextCursor: { createdAt: 100, eventId: target.id },
      hasMore: true,
    }),
  );
  projectChannelWindowMessages(client, "a");
  let mutation;
  function Harness() {
    mutation = useDeleteMessageMutation({ id: "a", channelType: "stream" });
    return null;
  }
  await act(async () =>
    env.root.render(
      React.createElement(
        QueryClientProvider,
        { client },
        React.createElement(Harness),
      ),
    ),
  );
  let result;
  await act(async () => {
    result = mutation.mutateAsync({ eventId: target.id });
    await started;
  });
  const before = client.getQueryData(key);
  let current = appendOlderChannelWindow(before, {
    startCursor: before.pages[0].nextCursor,
    rows: [{ event: event("older", 90), thread: null }],
    aux: [],
    nextCursor: null,
    hasMore: false,
  });
  current = mergeLiveChannelWindowEvent(current, event("concurrent-live", 110));
  client.setQueryData(key, current);
  projectChannelWindowMessages(client, "a");
  await act(async () => {
    acceptDelete();
    await result;
  });
  const after = client.getQueryData(key);
  assert.equal(
    after.pages.length,
    2,
    "deletion retains a concurrent older page",
  );
  assert.equal(after.revision, current.revision);
  assert.equal(after.pages[0].rows.length, 0);
  assert.equal(
    after.pages[0].nextCursor.eventId,
    target.id,
    "removing a boundary row must not invalidate its cursor",
  );
  client.setQueryData(
    key,
    mergeLiveChannelWindowEvent(after, event("live", 120)),
  );
  projectChannelWindowMessages(client, "a");
  assert.deepEqual(
    client.getQueryData(channelMessagesKey("a")).map((e) => e.id),
    ["older", "concurrent-live", "live"],
  );
});
