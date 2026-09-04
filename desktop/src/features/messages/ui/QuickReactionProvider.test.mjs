import assert from "node:assert/strict";
import { test } from "node:test";

import { JSDOM } from "jsdom";

const STORAGE_KEY = "buzz.quick-reaction-emojis.v1";
const storageKey = (scope) => (scope ? `${STORAGE_KEY}:${scope}` : STORAGE_KEY);
const entry = (emoji, count = 1, lastUsedAt = 1) => ({
  emoji,
  count,
  lastUsedAt,
});
const emojiNames = (items) => items.map((item) => item.emoji);

async function harness(
  t,
  {
    scope = "community-a",
    palette = [],
    recents = [],
    actionBars = false,
  } = {},
) {
  const dom = new JSDOM(
    "<!doctype html><html><body><div id='root'></div></body></html>",
    {
      url: "https://buzz.example.test",
    },
  );
  const globals = {
    window: dom.window,
    document: dom.window.document,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  };
  const originals = new Map(
    Object.keys(globals).map((key) => [
      key,
      Object.getOwnPropertyDescriptor(globalThis, key),
    ]),
  );
  Object.assign(globalThis, globals);
  let root;
  const clients = [];
  let act;
  t.after(async () => {
    try {
      if (root) await act(async () => root.unmount());
      for (const client of clients) client.clear();
    } finally {
      t.mock.restoreAll();
      dom.window.close();
      for (const [key, descriptor] of originals) {
        if (descriptor) Object.defineProperty(globalThis, key, descriptor);
        else delete globalThis[key];
      }
    }
  });

  // Import after installing the DOM: React Query and focus stores detect it at load time.
  const React = await import("react");
  ({ act } = React);
  const { createRoot } = await import("react-dom/client");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { customEmojiQueryKey } = await import("@/features/custom-emoji/hooks");
  const { QuickReactionProvider, useQuickReactionItems } = await import(
    "./QuickReactionProvider.tsx"
  );
  const { recordQuickReactionEmoji } = await import(
    "./useQuickReactionEmojis.ts"
  );
  // Real action bars keep their query hooks, quick buttons and closed menus.
  // Only fetching is disabled below; the production observer path is not mocked.
  const { MessageActionBar } = actionBars
    ? await import("./MessageActionBar.tsx")
    : {};
  const { TooltipProvider } = actionBars
    ? await import("@/shared/ui/tooltip.tsx")
    : {};
  const selectReaction = async () => {};
  function ActionBar({ id }) {
    return React.createElement(MessageActionBar, {
      message: {
        id: `message-${id}`,
        author: "Test author",
        pubkey: "a".repeat(64),
        createdAt: 1,
        time: "12:00",
        body: "Quick-reaction sharing regression",
        depth: 0,
        pending: false,
        kind: 9,
        tags: [],
      },
      reactions: [],
      onReactionSelect: selectReaction,
    });
  }
  const makeClient = (data) => {
    const client = new QueryClient({
      defaultOptions: {
        queries: {
          enabled: false,
          retry: false,
          gcTime: Infinity,
          // Retain instrumented fixture getters; still use the actual query and observer.
          structuralSharing: false,
        },
      },
    });
    client.setQueryData(customEmojiQueryKey, data);
    clients.push(client);
    return client;
  };
  const seed = (community, values) => {
    dom.window.localStorage.setItem(
      storageKey(community),
      JSON.stringify(values),
    );
  };
  const activate = (community) => {
    if (community)
      dom.window.localStorage.setItem("buzz-active-community-id", community);
    else dom.window.localStorage.removeItem("buzz-active-community-id");
  };
  seed(scope, recents);
  activate(scope);

  const storageListeners = new Set();
  const addListener = dom.window.addEventListener.bind(dom.window);
  const removeListener = dom.window.removeEventListener.bind(dom.window);
  t.mock.method(dom.window, "addEventListener", (type, callback, options) => {
    if (type === "storage") storageListeners.add(callback);
    return addListener(type, callback, options);
  });
  t.mock.method(
    dom.window,
    "removeEventListener",
    (type, callback, options) => {
      if (type === "storage") storageListeners.delete(callback);
      return removeListener(type, callback, options);
    },
  );

  const snapshots = new Map();
  function Consumer({ id, tick }) {
    const items = useQuickReactionItems();
    snapshots.set(id, items);
    return React.createElement(
      "output",
      { "data-consumer": id, "data-tick": tick },
      emojiNames(items).join(" "),
    );
  }
  const client = makeClient(palette);
  let options = {
    scope,
    client,
    count: 1,
    key: `${scope}:identity-a`,
    tick: 0,
  };
  root = createRoot(dom.window.document.getElementById("root"));
  const render = async (updates = {}) => {
    options = { ...options, ...updates, tick: options.tick + 1 };
    snapshots.clear();
    await act(async () => {
      root.render(
        React.createElement(
          QueryClientProvider,
          { client: options.client, key: options.key },
          React.createElement(
            QuickReactionProvider,
            { communityScope: options.scope },
            React.createElement(
              actionBars ? TooltipProvider : React.Fragment,
              null,
              Array.from({ length: options.count }, (_, id) =>
                React.createElement(actionBars ? ActionBar : Consumer, {
                  id,
                  key: id,
                  tick: options.tick,
                }),
              ),
            ),
          ),
        ),
      );
    });
  };
  const items = () => {
    assert.ok(
      snapshots.has(0),
      "the real provider must publish a consumer snapshot",
    );
    return snapshots.get(0);
  };
  const updatePalette = async (data, target = options.client) => {
    await act(async () => {
      target.setQueryData(customEmojiQueryKey, data);
      // Drain React Query's scheduled notification, not a timing/performance assertion.
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };
  const storageEvent = async (community) => {
    await act(async () => {
      dom.window.dispatchEvent(
        new dom.window.StorageEvent("storage", {
          key: storageKey(community),
          newValue: dom.window.localStorage.getItem(storageKey(community)),
          storageArea: dom.window.localStorage,
        }),
      );
    });
  };
  return {
    act,
    activate,
    client,
    dom,
    items,
    makeClient,
    recordQuickReactionEmoji,
    render,
    seed,
    snapshots,
    storageEvent,
    storageListeners,
    updatePalette,
    query: (target = client) =>
      target.getQueryCache().find({ queryKey: customEmojiQueryKey }),
    async unmount() {
      await act(async () => root.unmount());
      root = undefined;
    },
  };
}

test("many quick-reaction consumers reuse one preparation, query observer and storage listener", async (t) => {
  let paletteReads = 0;
  const palette = Array.from({ length: 128 }, (_, index) => ({
    get shortcode() {
      paletteReads += 1;
      return index === 0 ? "shipit" : `custom_${index}`;
    },
    url: `https://cdn.example.test/emoji/${index}.png`,
  }));
  const h = await harness(t, { palette, recents: [entry(":shipit:", 4)] });
  await h.render();
  assert.deepEqual(emojiNames(h.items()), [":shipit:", "👍", "❤️"]);
  assert.equal(h.items()[0].customEmojiUrl, palette[0].url);
  const prepared = h.items();
  const readsAfterPreparation = paletteReads;
  assert.ok(
    readsAfterPreparation > 0,
    "the production preparation must inspect the palette",
  );
  await h.render({ count: 64 });
  await h.render();
  assert.equal(h.snapshots.size, 64);
  for (const items of h.snapshots.values()) assert.strictEqual(items, prepared);
  assert.equal(
    paletteReads,
    readsAfterPreparation,
    "row mounts/rerenders must not repeat palette preparation",
  );
  assert.equal(h.query().getObserversCount(), 1);
  assert.equal(h.storageListeners.size, 1);
  assert.equal(h.dom.window.document.querySelectorAll("output").length, 64);
  // A refetch may replace the palette array without changing any tray item.
  await h.updatePalette([
    { shortcode: "shipit", url: palette[0].url },
    { shortcode: "unrelated", url: "https://cdn.example.test/unrelated.png" },
  ]);
  assert.strictEqual(h.items(), prepared);
  await h.storageEvent("community-a");
  assert.strictEqual(h.items(), prepared);
  await h.unmount();
  assert.equal(h.query().getObserversCount(), 0);
  assert.equal(h.storageListeners.size, 0);
});

test("real MessageActionBars share quick-tray preparation, observer and storage listener", async (t) => {
  let paletteReads = 0;
  const palette = Array.from({ length: 128 }, (_, index) => ({
    get shortcode() {
      paletteReads += 1;
      return index === 0 ? "shipit" : `custom_${index}`;
    },
    url: `https://cdn.example.test/emoji/${index}.png`,
  }));
  const h = await harness(t, {
    actionBars: true,
    palette,
    recents: [entry(":shipit:", 4)],
  });
  await h.render();
  const readsAfterPreparation = paletteReads;
  assert.ok(readsAfterPreparation > 0);
  for (const count of [16, 16, 1, 16]) {
    await h.render({ count });
    const bars = h.dom.window.document.querySelectorAll(
      '[data-testid^="message-action-bar-"]',
    );
    assert.equal(bars.length, count, "mount the real production action bars");
    for (const bar of bars) {
      const buttons = bar.querySelectorAll('button[aria-label^="React with "]');
      assert.equal(buttons.length, 3, "render the actual quick buttons");
      assert.equal(
        buttons[0].querySelector("img")?.getAttribute("alt"),
        ":shipit:",
      );
      assert.equal(
        buttons[0].querySelector("img")?.getAttribute("src"),
        palette[0].url,
      );
    }
    assert.equal(
      h.query().getObserversCount(),
      1,
      "real rows must not add palette observers",
    );
    assert.equal(
      h.storageListeners.size,
      1,
      "real rows must not add storage listeners",
    );
    assert.equal(
      paletteReads,
      readsAfterPreparation,
      "real row mounts/rerenders must not prepare the palette",
    );
  }
  await h.unmount();
  assert.equal(h.query().getObserversCount(), 0);
  assert.equal(h.storageListeners.size, 0);
});

test("recording persists recents without reshuffling mounted or newly mounted trays", async (t) => {
  const h = await harness(t, {
    palette: [{ shortcode: "shipit", url: "https://cdn.example.test/old.png" }],
    recents: [entry("👍", 2), entry(":shipit:", 4), entry("🔥", 3)],
  });
  await h.render({ count: 4 });
  const prepared = h.items();
  await h.act(async () => {
    for (let i = 0; i < 6; i++) h.recordQuickReactionEmoji(" 🚀 ");
  });
  const saved = JSON.parse(
    h.dom.window.localStorage.getItem(storageKey("community-a")),
  );
  assert.equal(saved[0].emoji, "🚀");
  assert.equal(saved[0].count, 6);
  await h.render({ count: 20 });
  for (const items of h.snapshots.values()) assert.strictEqual(items, prepared);
  await h.updatePalette([
    { shortcode: "shipit", url: "https://cdn.example.test/new.png" },
  ]);
  assert.deepEqual(emojiNames(h.items()), [":shipit:", "🔥", "👍"]);
  assert.equal(h.items()[0].customEmojiUrl, "https://cdn.example.test/new.png");
  for (const items of h.snapshots.values())
    assert.strictEqual(items, h.items());
  // A fresh identity/session owner reads persisted history rather than a module-global tray.
  await h.render({ key: "community-a:identity-b" });
  assert.deepEqual(emojiNames(h.items()), ["🚀", ":shipit:", "🔥"]);
});

test("storage refresh is community-scoped and removal restores defaults", async (t) => {
  const h = await harness(t, { recents: [entry("🔥", 4)] });
  await h.render({ count: 8 });
  const prepared = h.items();
  h.seed("community-a", [entry("✅", 10, 1), entry("🚀", 10, 2)]);
  h.seed("community-b", [entry("🧊", 12)]);
  await h.storageEvent("community-b");
  assert.strictEqual(h.items(), prepared);
  await h.storageEvent("community-a");
  assert.deepEqual(emojiNames(h.items()), ["🚀", "✅", "👍"]);
  for (const items of h.snapshots.values())
    assert.strictEqual(items, h.items());
  h.dom.window.localStorage.removeItem(storageKey("community-a"));
  await h.storageEvent("community-a");
  assert.deepEqual(emojiNames(h.items()), ["👍", "❤️", "😂"]);
});

test("palette availability backfills stale custom emoji and restores their frozen rank", async (t) => {
  const h = await harness(t, {
    recents: [
      entry(":gone:", 20),
      entry(":SHIPIT:", 10),
      entry(":SHIPIT:", 9),
      entry("🔥", 8),
      entry("👍", 7),
    ],
  });
  await h.render();
  assert.deepEqual(emojiNames(h.items()), ["🔥", "👍", "❤️"]);
  const available = [
    { shortcode: "shipit", url: "https://cdn.example.test/shipit.png" },
  ];
  await h.updatePalette(available);
  assert.deepEqual(emojiNames(h.items()), [":SHIPIT:", "🔥", "👍"]);
  assert.equal(h.items()[0].customEmojiUrl, available[0].url);
  await h.updatePalette([]);
  assert.deepEqual(emojiNames(h.items()), ["🔥", "👍", "❤️"]);
  assert.ok(h.items().every((item) => item.customEmojiUrl === undefined));
  await h.updatePalette(available);
  assert.deepEqual(emojiNames(h.items()), [":SHIPIT:", "🔥", "👍"]);
});

test("keyed community replacement releases old observers/listeners and cannot reuse old custom URLs", async (t) => {
  const h = await harness(t, {
    palette: [
      { shortcode: "shipit", url: "https://a.example.test/shipit.png" },
    ],
    recents: [entry(":shipit:", 5)],
  });
  await h.render({ count: 16 });
  const aListener = [...h.storageListeners][0];
  const b = h.makeClient([
    { shortcode: "shipit", url: "https://b.example.test/shipit.png" },
  ]);
  h.seed("community-b", [entry("✅", 10), entry(":shipit:", 5)]);
  h.activate("community-b");
  await h.render({
    scope: "community-b",
    key: "community-b:identity-a",
    client: b,
  });
  assert.deepEqual(emojiNames(h.items()), ["✅", ":shipit:", "👍"]);
  assert.equal(
    h.items()[1].customEmojiUrl,
    "https://b.example.test/shipit.png",
  );
  assert.equal(h.query().getObserversCount(), 0);
  assert.equal(h.query(b).getObserversCount(), 1);
  assert.equal(h.storageListeners.size, 1);
  assert.ok(!h.storageListeners.has(aListener));
  const bItems = h.items();
  h.seed("community-a", [entry("🧊", 100)]);
  await h.storageEvent("community-a");
  await h.updatePalette([], h.client);
  assert.strictEqual(h.items(), bItems);
  await h.unmount();
  assert.equal(h.storageListeners.size, 0);
  assert.equal(h.query(b).getObserversCount(), 0);
});

test("null community scope reads and refreshes the legacy unscoped history", async (t) => {
  const h = await harness(t, { scope: null, recents: [entry("🚀", 3)] });
  await h.render();
  assert.deepEqual(emojiNames(h.items()), ["🚀", "👍", "❤️"]);
  h.seed(null, [entry("✅", 5)]);
  await h.storageEvent(null);
  assert.deepEqual(emojiNames(h.items()), ["✅", "👍", "❤️"]);
});

test("malformed history and inaccessible storage keep a usable default tray", async (t) => {
  const h = await harness(t);
  h.dom.window.localStorage.setItem(storageKey("community-a"), "{not json");
  await h.render();
  assert.deepEqual(emojiNames(h.items()), ["👍", "❤️", "😂"]);
  h.dom.window.localStorage.setItem(
    storageKey("community-a"),
    JSON.stringify({ emoji: "🔥" }),
  );
  await h.storageEvent("community-a");
  assert.deepEqual(emojiNames(h.items()), ["👍", "❤️", "😂"]);
  const descriptor = Object.getOwnPropertyDescriptor(
    h.dom.window,
    "localStorage",
  );
  Object.defineProperty(h.dom.window, "localStorage", {
    configurable: true,
    get() {
      throw new h.dom.window.DOMException("Storage denied", "SecurityError");
    },
  });
  try {
    await h.render({ key: "community-a:storage-denied" });
    assert.deepEqual(emojiNames(h.items()), ["👍", "❤️", "😂"]);
    assert.doesNotThrow(() => h.recordQuickReactionEmoji("🔥"));
  } finally {
    Object.defineProperty(h.dom.window, "localStorage", descriptor);
  }
});
