import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { after, beforeEach, test } from "node:test";
import { JSDOM } from "jsdom";

// Keep both production hooks, destination resolution and the durable drain real;
// substitute only navigation and Tauri IPC to observe their boundary contracts.
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "@/app/navigation/useAppNavigation") {
      return { shortCircuit: true, url: "buzz-test:navigation" };
    }
    return next(specifier, context);
  },
  load(url, context, next) {
    if (url === "buzz-test:navigation") {
      return {
        shortCircuit: true,
        format: "module",
        source:
          "export function useAppNavigation() { return globalThis.testNavigation; }",
      };
    }
    return next(url, context);
  },
});
const dom = new JSDOM("<!doctype html><body></body>", {
  url: "http://localhost",
});
Object.assign(globalThis, {
  window: dom.window,
  document: dom.window.document,
  IS_REACT_ACT_ENVIRONMENT: true,
});
const { createElement, act } = await import("react");
const { createRoot } = await import("react-dom/client");
const { useOpenMessageLink } = await import("./useOpenMessageLink.ts");
const { useMessageDeepLinks } = await import("@/shared/useMessageDeepLinks");
const { resetNavigationDeepLinkDrain } = await import("@/shared/deep-link");
let queue, calls, accepted, fetchEvent, navigate;
const ipc = {
  transformCallback: () => 1,
  invoke: async (command, args) => {
    if (command === "clear_pending_navigation_deep_links") return;
    if (command === "get_event") return JSON.stringify(await fetchEvent(args));
    if (command === "take_pending_navigation_deep_link")
      return queue[0] ?? null;
    if (command === "acknowledge_pending_navigation_deep_link") {
      accepted.push(args.id);
      queue.shift();
      return true;
    }
    if (command.startsWith("plugin:event|")) return 1;
    throw new Error(`Unexpected IPC: ${command}`);
  },
};
globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__ = ipc;
dom.window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
const link = { channelId: "channel", messageId: "message", threadRootId: null };
function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
async function settle() {
  await act(async () => {
    await new Promise((done) => setTimeout(done, 10));
  });
}
async function mount(listener = false) {
  let open;
  function Harness({ enabled }) {
    open = useOpenMessageLink();
    useMessageDeepLinks(listener && enabled);
    return null;
  }
  const container = document.createElement("div");
  const root = createRoot(container);
  await act(async () => root.render(createElement(Harness, { enabled: true })));
  return {
    open: (...args) => open(...args),
    disable: async () =>
      act(async () => root.render(createElement(Harness, { enabled: false }))),
    unmount: async () => act(async () => root.unmount()),
  };
}
beforeEach(async () => {
  await resetNavigationDeepLinkDrain();
  queue = [];
  calls = [];
  accepted = [];
  fetchEvent = async () => ({ kind: 45001, tags: [] });
  navigate = async () => true;
  globalThis.testNavigation = {
    goChannel: (...args) => {
      calls.push(["channel", ...args]);
      return navigate();
    },
    goForumPost: (...args) => {
      calls.push(["forum", ...args]);
      return navigate();
    },
  };
});
after(() => dom.window.close());
for (const [kind, expected] of [
  [45001, ["forum", "channel", "message", { replyId: undefined }]],
  [45003, ["forum", "channel", "a".repeat(64), { replyId: "message" }]],
  [9, ["channel", "channel", { messageId: "message", threadRootId: null }]],
]) {
  test(`production hook routes event kind ${kind}`, async () => {
    fetchEvent = async () => ({
      kind,
      tags: [
        ["e", "a".repeat(64), "", "root"],
        ["e", "a".repeat(64), "", "reply"],
      ],
    });
    const app = await mount();
    try {
      assert.equal(await app.open(link), true);
      assert.deepEqual(calls, [expected]);
    } finally {
      await app.unmount();
    }
  });
}
for (const kind of [45001, 9]) {
  test(`refused kind ${kind} stays queued until an accepted retry`, async () => {
    fetchEvent = async () => ({ kind, tags: [] });
    queue.push({ id: "pending", kind: "message", ...link });
    navigate = async () => false;
    const app = await mount(true);
    await settle();
    assert.equal(calls.length, 1);
    assert.deepEqual(accepted, []);
    assert.equal(queue.length, 1);
    await app.unmount();
    navigate = async () => true;
    const retry = await mount(true);
    try {
      await settle();
      assert.deepEqual(accepted, ["pending"]);
      assert.equal(queue.length, 0);
    } finally {
      await retry.unmount();
    }
  });
}
test("acknowledgement waits for navigation completion", async () => {
  const pending = deferred();
  navigate = () => pending.promise;
  queue.push({ id: "pending", kind: "message", ...link });
  const app = await mount(true);
  try {
    await settle();
    assert.equal(calls.length, 1);
    assert.deepEqual(accepted, []);
    pending.resolve(true);
    await settle();
    assert.deepEqual(accepted, ["pending"]);
  } finally {
    await app.unmount();
  }
});
for (const lifecycle of ["unmount", "disable"]) {
  test(`${lifecycle} during lookup prevents stale navigation and acknowledgement`, async () => {
    const lookup = deferred();
    fetchEvent = () => lookup.promise;
    queue.push({ id: "pending", kind: "message", ...link });
    const app = await mount(true);
    await settle();
    await app[lifecycle]();
    lookup.resolve({ kind: 45001, tags: [] });
    await settle();
    assert.deepEqual(calls, []);
    assert.deepEqual(accepted, []);
    if (lifecycle === "disable") await app.unmount();
  });
}
test("in-app hook lookup cannot navigate after community unmount", async () => {
  const lookup = deferred();
  fetchEvent = () => lookup.promise;
  const app = await mount();
  const result = app.open(link);
  await app.unmount();
  lookup.resolve({ kind: 9, tags: [] });
  assert.equal(await result, false);
  assert.deepEqual(calls, []);
});

for (const message of ["relay unavailable", "event not found"]) {
  test(`failed lookup (${message}) stays queued and retries the forum destination`, async () => {
    fetchEvent = async () => {
      throw new Error(message);
    };
    queue.push({ id: "pending", kind: "message", ...link });
    const app = await mount(true);
    try {
      await settle();
      assert.deepEqual(calls, []);
      assert.deepEqual(accepted, []);
      assert.equal(queue.length, 1);
    } finally {
      await app.unmount();
    }
    fetchEvent = async () => ({ kind: 45001, tags: [] });
    const retry = await mount(true);
    try {
      await settle();
      assert.deepEqual(calls, [
        ["forum", "channel", "message", { replyId: undefined }],
      ]);
      assert.deepEqual(accepted, ["pending"]);
    } finally {
      await retry.unmount();
    }
  });
}

test("in-app failed lookup keeps its best-effort channel fallback", async () => {
  fetchEvent = async () => {
    throw new Error("relay unavailable");
  };
  const app = await mount();
  try {
    assert.equal(await app.open(link), true);
    assert.deepEqual(calls, [
      ["channel", "channel", { messageId: "message", threadRootId: null }],
    ]);
  } finally {
    await app.unmount();
  }
});

for (const olderFails of [false, true]) {
  test(`latest activation wins when an older lookup ${olderFails ? "fails" : "succeeds"} late`, async () => {
    const older = deferred();
    fetchEvent = async ({ eventId }) => {
      if (eventId === "message") {
        await older.promise;
        if (olderFails) throw new Error("old lookup failed");
      }
      return { kind: 45001, tags: [] };
    };
    const app = await mount();
    try {
      const first = app.open(link);
      assert.equal(await app.open({ ...link, messageId: "newer" }), true);
      older.resolve();
      assert.equal(await first, false);
      assert.deepEqual(calls, [
        ["forum", "channel", "newer", { replyId: undefined }],
      ]);
    } finally {
      await app.unmount();
    }
  });
}

test("a newer activation in another mounted renderer supersedes the old lookup", async () => {
  const older = deferred();
  fetchEvent = async ({ eventId }) => {
    if (eventId === "message") await older.promise;
    return { kind: 45001, tags: [] };
  };
  const firstRenderer = await mount();
  const secondRenderer = await mount();
  try {
    const first = firstRenderer.open(link);
    assert.equal(
      await secondRenderer.open({ ...link, messageId: "newer" }),
      true,
    );
    older.resolve();
    assert.equal(await first, false);
    assert.deepEqual(calls, [
      ["forum", "channel", "newer", { replyId: undefined }],
    ]);
  } finally {
    await firstRenderer.unmount();
    await secondRenderer.unmount();
  }
});

test("community reset invalidates a lookup even before hook cleanup", async () => {
  const { resetMessageLinkRequests } = await import("./messageLinkRequests.ts");
  const lookup = deferred();
  fetchEvent = () => lookup.promise;
  const app = await mount();
  try {
    const pending = app.open(link);
    resetMessageLinkRequests();
    lookup.resolve({ kind: 45001, tags: [] });
    assert.equal(await pending, false);
    assert.deepEqual(calls, []);
    assert.equal(await app.open({ ...link, messageId: "new-community" }), true);
  } finally {
    await app.unmount();
  }
});

test("a queued forum comment without a root is not acknowledged as a channel visit", async () => {
  fetchEvent = async () => ({ kind: 45003, tags: [] });
  queue.push({ id: "pending", kind: "message", ...link });
  const app = await mount(true);
  try {
    await settle();
    assert.deepEqual(calls, []);
    assert.deepEqual(accepted, []);
    assert.equal(queue.length, 1);
  } finally {
    await app.unmount();
  }
});
