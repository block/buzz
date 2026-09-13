import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { afterEach, test } from "node:test";

const ipcHandlers = new Map();
let nextCallbackId = 1;
const callbacks = new Map();

const tauriInternals = {
  invoke: (cmd, args) => {
    const handler = ipcHandlers.get(cmd);
    if (handler) return Promise.resolve(handler(args));
    return Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
  },
  transformCallback: (callback) => {
    const id = nextCallbackId++;
    callbacks.set(id, callback);
    return id;
  },
};
globalThis.window = {
  __TAURI_INTERNALS__: tauriInternals,
  __TAURI_EVENT_PLUGIN_INTERNALS__: { unregisterListener: () => {} },
};
globalThis.__TAURI_INTERNALS__ = tauriInternals;

const {
  listenForEntityDeepLinks,
  listenForNavigationDeepLinks,
  resetNavigationDeepLinkDrain,
} = await import("@/shared/deep-link.ts");

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

afterEach(() => {
  ipcHandlers.clear();
  callbacks.clear();
});

test("listener teardown leaves an unaccepted FIFO item for the next mount", async () => {
  const queue = [
    {
      id: "first",
      kind: "channel",
      channelId: "channel-1",
      messageId: null,
      threadRootId: null,
    },
    {
      id: "second",
      kind: "message",
      channelId: "channel-2",
      messageId: "message-2",
      threadRootId: "root-2",
    },
  ];
  const firstAcknowledge = deferred();
  const acknowledged = [];
  let unlistenCount = 0;

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {
    unlistenCount += 1;
  });
  ipcHandlers.set("take_pending_navigation_deep_link", () => queue[0] ?? null);
  ipcHandlers.set(
    "acknowledge_pending_navigation_deep_link",
    async ({ id }) => {
      if (id === "first") await firstAcknowledge.promise;
      assert.equal(queue[0]?.id, id);
      acknowledged.push(id);
      queue.shift();
      return true;
    },
  );

  let firstMountActive = true;
  const firstOpened = [];
  const firstUnlisten = await listenForNavigationDeepLinks(
    (payload) => {
      if (!firstMountActive) return false;
      firstOpened.push(payload.channelId);
      return true;
    },
    (payload) => {
      if (!firstMountActive) return false;
      firstOpened.push(payload.messageId);
      return true;
    },
  );
  await settle();
  assert.deepEqual(firstOpened, ["channel-1"]);

  firstMountActive = false;
  firstUnlisten();
  firstAcknowledge.resolve();
  await settle();

  assert.deepEqual(acknowledged, ["first"]);
  assert.equal(queue[0]?.id, "second");

  const secondOpened = [];
  const secondUnlisten = await listenForNavigationDeepLinks(
    (payload) => {
      secondOpened.push(payload.channelId);
      return true;
    },
    (payload) => {
      secondOpened.push(payload.messageId);
      return true;
    },
  );
  await settle();

  assert.deepEqual(secondOpened, ["message-2"]);
  assert.deepEqual(acknowledged, ["first", "second"]);
  assert.equal(queue.length, 0);
  secondUnlisten();
  assert.equal(unlistenCount, 4);
});

test("concurrent listener remount does not take or acknowledge the in-flight head twice", async () => {
  const queue = [
    {
      id: "in-flight",
      kind: "channel",
      channelId: "channel-1",
      messageId: null,
      threadRootId: null,
    },
  ];
  const acknowledgeGate = deferred();
  const opened = [];
  const acknowledged = [];

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {});
  ipcHandlers.set("take_pending_navigation_deep_link", () => queue[0] ?? null);
  ipcHandlers.set(
    "acknowledge_pending_navigation_deep_link",
    async ({ id }) => {
      acknowledged.push(id);
      await acknowledgeGate.promise;
      assert.equal(queue[0]?.id, id);
      queue.shift();
      return true;
    },
  );

  const firstUnlisten = await listenForNavigationDeepLinks(
    (payload) => {
      opened.push(`first:${payload.channelId}`);
      return true;
    },
    () => true,
  );
  await settle();
  assert.deepEqual(opened, ["first:channel-1"]);
  assert.deepEqual(acknowledged, ["in-flight"]);

  firstUnlisten();
  const secondUnlisten = await listenForNavigationDeepLinks(
    (payload) => {
      opened.push(`second:${payload.channelId}`);
      return true;
    },
    () => true,
  );
  await settle();

  assert.deepEqual(opened, ["first:channel-1"]);
  assert.deepEqual(acknowledged, ["in-flight"]);

  acknowledgeGate.resolve();
  await settle();
  await settle();

  assert.deepEqual(opened, ["first:channel-1"]);
  assert.deepEqual(acknowledged, ["in-flight"]);
  assert.equal(queue.length, 0);
  secondUnlisten();
});

test("community reset prevents an in-flight route from acknowledging", async () => {
  const pending = {
    id: "old-community",
    kind: "channel",
    channelId: "channel-1",
    messageId: null,
    threadRootId: null,
  };
  const routeGate = deferred();
  let acknowledgeCount = 0;
  let clearCount = 0;

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {});
  ipcHandlers.set("clear_pending_navigation_deep_links", () => {
    clearCount += 1;
  });
  ipcHandlers.set("take_pending_navigation_deep_link", () => pending);
  ipcHandlers.set("acknowledge_pending_navigation_deep_link", () => {
    acknowledgeCount += 1;
    return true;
  });

  const unlisten = await listenForNavigationDeepLinks(
    async () => {
      await routeGate.promise;
      return true;
    },
    () => true,
  );
  await settle();

  await resetNavigationDeepLinkDrain();
  routeGate.resolve();
  await settle();

  assert.equal(clearCount, 1);
  assert.equal(acknowledgeCount, 0);
  unlisten();
});

test("community reset after take does not route the stale item", async () => {
  const takeGate = deferred();
  const opened = [];

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {});
  ipcHandlers.set("clear_pending_navigation_deep_links", () => {});
  ipcHandlers.set("take_pending_navigation_deep_link", async () => {
    await takeGate.promise;
    return {
      id: "old-community",
      kind: "channel",
      channelId: "channel-1",
      messageId: null,
      threadRootId: null,
    };
  });
  ipcHandlers.set("acknowledge_pending_navigation_deep_link", () => true);

  const unlisten = await listenForNavigationDeepLinks(
    (payload) => {
      opened.push(payload.channelId);
      return true;
    },
    () => true,
  );
  await settle();

  await resetNavigationDeepLinkDrain();
  takeGate.resolve();
  await settle();

  assert.deepEqual(opened, []);
  unlisten();
});

test("community reset stops the stale drain before taking another item", async () => {
  const queue = [
    {
      id: "first",
      kind: "channel",
      channelId: "channel-1",
      messageId: null,
      threadRootId: null,
    },
    {
      id: "second",
      kind: "channel",
      channelId: "channel-2",
      messageId: null,
      threadRootId: null,
    },
  ];
  const opened = [];
  let takeCount = 0;

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {});
  ipcHandlers.set("clear_pending_navigation_deep_links", () => {
    queue.length = 0;
  });
  ipcHandlers.set("take_pending_navigation_deep_link", () => {
    takeCount += 1;
    return queue[0] ?? null;
  });
  ipcHandlers.set(
    "acknowledge_pending_navigation_deep_link",
    async ({ id }) => {
      assert.equal(queue[0]?.id, id);
      queue.shift();
      await resetNavigationDeepLinkDrain();
      return true;
    },
  );

  const unlisten = await listenForNavigationDeepLinks(
    (payload) => {
      opened.push(payload.channelId);
      return true;
    },
    () => true,
  );
  await settle();
  await settle();

  assert.deepEqual(opened, ["channel-1"]);
  assert.equal(takeCount, 1);
  unlisten();
});

test("failed community clear quarantines stale navigation from the next listener", async () => {
  const stale = {
    id: "old-community",
    kind: "channel",
    channelId: "old-channel",
    messageId: null,
    threadRootId: null,
  };
  const opened = [];
  let takeCount = 0;

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {});
  ipcHandlers.set("clear_pending_navigation_deep_links", () => {
    throw new Error("clear failed");
  });
  ipcHandlers.set("take_pending_navigation_deep_link", () => {
    takeCount += 1;
    return stale;
  });

  await assert.rejects(resetNavigationDeepLinkDrain(), /clear failed/);
  const unlisten = await listenForNavigationDeepLinks(
    (payload) => {
      opened.push(payload.channelId);
      return true;
    },
    () => true,
  );
  await settle();

  assert.equal(takeCount, 0);
  assert.deepEqual(opened, []);
  unlisten();

  // Restore the module-level gate for later tests, just as a successful retry
  // does in the application.
  ipcHandlers.set("clear_pending_navigation_deep_links", () => {});
  await resetNavigationDeepLinkDrain();
});

test("refused navigation stays at the FIFO head and retries with one acknowledgement", async () => {
  const pending = {
    id: "retry-me",
    kind: "message",
    channelId: "channel-1",
    messageId: "message-1",
    threadRootId: "root-1",
  };
  const queue = [pending];
  const opened = [];
  const acknowledged = [];

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {});
  ipcHandlers.set("take_pending_navigation_deep_link", () => queue[0] ?? null);
  ipcHandlers.set("acknowledge_pending_navigation_deep_link", ({ id }) => {
    assert.equal(queue[0]?.id, id);
    acknowledged.push(id);
    queue.shift();
    return true;
  });

  const firstUnlisten = await listenForNavigationDeepLinks(
    () => true,
    (payload) => {
      opened.push(`refused:${payload.messageId}`);
      return false;
    },
  );
  await settle();

  assert.deepEqual(opened, ["refused:message-1"]);
  assert.equal(queue[0], pending);
  assert.deepEqual(acknowledged, []);
  firstUnlisten();

  const secondUnlisten = await listenForNavigationDeepLinks(
    () => true,
    (payload) => {
      opened.push(`accepted:${payload.messageId}`);
      return true;
    },
  );
  await settle();
  await settle();

  assert.deepEqual(opened, ["refused:message-1", "accepted:message-1"]);
  assert.deepEqual(acknowledged, ["retry-me"]);
  assert.equal(queue.length, 0);
  secondUnlisten();
});

test("rejected navigation remains queued and is not acknowledged", async () => {
  const pending = {
    id: "retry-me",
    kind: "channel",
    channelId: "channel-1",
    messageId: null,
    threadRootId: null,
  };
  let acknowledgeCount = 0;
  const warnings = [];
  const originalWarn = console.warn;

  ipcHandlers.set("plugin:event|listen", () => nextCallbackId);
  ipcHandlers.set("plugin:event|unlisten", () => {});
  ipcHandlers.set("take_pending_navigation_deep_link", () => pending);
  ipcHandlers.set("acknowledge_pending_navigation_deep_link", () => {
    acknowledgeCount += 1;
    return true;
  });
  console.warn = (...args) => warnings.push(args);

  try {
    const unlisten = await listenForNavigationDeepLinks(
      async () => {
        throw new Error("route failed");
      },
      async () => true,
    );
    await settle();

    assert.equal(acknowledgeCount, 0);
    assert.equal(warnings.length, 1);
    assert.match(String(warnings[0][1]), /route failed/);
    unlisten();
  } finally {
    console.warn = originalWarn;
  }
});

for (const delivery of ["cold-start", "running-instance"]) {
  test(`canonical native demo entity payloads navigate before acknowledgement: ${delivery}`, async () => {
    // Rust's canonical_entity_deep_link test proves all demo transport URLs
    // produce these exact shared values, rather than passing demo schemes on.
    const golden = JSON.parse(
      readFileSync(
        new URL("../../../test-fixtures/entity-links.json", import.meta.url),
        "utf8",
      ),
    );
    const { parseEntityLink } = await import("@/shared/lib/entityLink.ts");
    const pending = Object.entries(golden.links).map(([id, href]) => ({
      id,
      href,
    }));
    const queue = delivery === "cold-start" ? [...pending] : [];
    const opened = [];
    const acknowledged = [];
    let notify;
    ipcHandlers.set("plugin:event|listen", ({ handler }) => {
      notify = callbacks.get(handler);
      return handler;
    });
    ipcHandlers.set("plugin:event|unlisten", () => {});
    ipcHandlers.set("take_pending_entity_deep_link", () => queue[0] ?? null);
    ipcHandlers.set("acknowledge_pending_entity_deep_link", ({ id }) => {
      assert.equal(queue[0]?.id, id);
      assert.equal(opened.length, acknowledged.length + 1);
      acknowledged.push(id);
      queue.shift();
      return true;
    });
    const unlisten = await listenForEntityDeepLinks((href) => {
      const parsed = parseEntityLink(href);
      assert.equal(parsed.ok, true, href);
      opened.push(parsed.value);
      return true;
    });
    await settle();
    if (delivery === "running-instance") {
      queue.push(...pending);
      notify({ payload: pending[0] });
    }
    await settle();
    await settle();
    assert.deepEqual(
      acknowledged,
      pending.map(({ id }) => id),
    );
    assert.equal(opened.length, pending.length);
    assert.equal(queue.length, 0);
    unlisten();
  });
}
