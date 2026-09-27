import assert from "node:assert/strict";
import { afterEach, before, test } from "node:test";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

before(() => {
  installLocalStorage();
});

afterEach(async () => {
  const { resetConversationPlaygroundPins } = await import(
    "../../playground/lib/conversationPins.ts"
  );
  resetConversationPlaygroundPins();
  globalThis.localStorage?.clear();
});

test("buildBrowserRowPinCard keeps sid and normalizes URL/name", async () => {
  const { buildBrowserRowPinCard } = await import(
    "./pinBrowserToConversation.ts"
  );

  const card = buildBrowserRowPinCard({
    sid: "browser-1",
    title: "Example",
    url: "example.com/path",
    name: "  Custom  ",
  });
  assert.ok(card);
  assert.equal(card.sid, "browser-1");
  assert.equal(card.url, "https://example.com/path");
  assert.equal(card.name, "Custom");
  assert.equal(card.hula, "playground");
});

test("buildBrowserRowPinCard rejects empty sid or bad URL", async () => {
  const { buildBrowserRowPinCard } = await import(
    "./pinBrowserToConversation.ts"
  );

  assert.equal(
    buildBrowserRowPinCard({ sid: "", title: "X", url: "https://a.com" }),
    null,
  );
  assert.equal(
    buildBrowserRowPinCard({
      sid: "s1",
      title: "X",
      url: "ftp://nope.example",
    }),
    null,
  );
});

test("resolveBrowserRowPinTarget scopes channel vs thread", async () => {
  const { resolveBrowserRowPinTarget } = await import(
    "./pinBrowserToConversation.ts"
  );

  assert.deepEqual(
    resolveBrowserRowPinTarget({ channelId: "chan-a" }),
    {
      scopeKey: "channel:chan-a",
      channelId: "chan-a",
      threadRoot: null,
    },
  );
  assert.deepEqual(
    resolveBrowserRowPinTarget({
      channelId: "chan-a",
      threadRoot: "root-1",
    }),
    {
      scopeKey: "thread:root-1",
      channelId: "chan-a",
      threadRoot: "root-1",
    },
  );
  assert.equal(resolveBrowserRowPinTarget({ channelId: "  " }), null);
});

test("pinBrowserRowToConversation writes conversationPins for that sid", async () => {
  const { pinBrowserRowToConversation } = await import(
    "./pinBrowserToConversation.ts"
  );
  const {
    hasConversationPlaygroundPin,
    listConversationPinBindingsForSid,
    listConversationPlaygroundPins,
    unpinPlaygroundFromConversation,
  } = await import("../../playground/lib/conversationPins.ts");

  const pinned = pinBrowserRowToConversation({
    sid: "browser-1",
    title: "Demo",
    url: "https://app.example.com",
    channelId: "chan-a",
    threadRoot: "root-1",
  });
  assert.ok(pinned);
  assert.equal(pinned.sid, "browser-1");
  assert.equal(pinned.channelId, "chan-a");
  assert.equal(pinned.url, "https://app.example.com/");
  assert.equal(
    hasConversationPlaygroundPin("thread:root-1", "browser-1"),
    true,
  );
  assert.equal(listConversationPlaygroundPins("thread:root-1").length, 1);

  const bindings = listConversationPinBindingsForSid("browser-1");
  assert.equal(bindings.length, 1);
  assert.equal(bindings[0]?.scopeKey, "thread:root-1");
  assert.equal(bindings[0]?.channelId, "chan-a");
  assert.equal(bindings[0]?.threadRoot, "root-1");

  // Editable URL override on re-pin
  pinBrowserRowToConversation({
    sid: "browser-1",
    title: "Demo",
    url: "https://other.example.com/x",
    name: "Other",
    channelId: "chan-a",
    threadRoot: "root-1",
  });
  assert.equal(
    listConversationPlaygroundPins("thread:root-1")[0]?.url,
    "https://other.example.com/x",
  );
  assert.equal(
    listConversationPlaygroundPins("thread:root-1")[0]?.name,
    "Other",
  );

  // Unpin from conversation removes the binding (Browsers chip sync).
  unpinPlaygroundFromConversation("thread:root-1", "browser-1");
  assert.equal(listConversationPinBindingsForSid("browser-1").length, 0);
});

test("listBrowserRowPinBindings unions tab sids and dedupes scope", async () => {
  const { listBrowserRowPinBindings, pinBrowserRowToConversation } =
    await import("./pinBrowserToConversation.ts");

  pinBrowserRowToConversation({
    sid: "main-1",
    title: "Main",
    url: "https://a.example",
    channelId: "chan-a",
  });
  pinBrowserRowToConversation({
    sid: "tab-2",
    title: "Tab",
    url: "https://b.example",
    channelId: "chan-b",
    threadRoot: "t1",
  });
  // Same scope via active surface (should dedupe)
  pinBrowserRowToConversation({
    sid: "main-1",
    title: "Main",
    url: "https://a.example",
    channelId: "chan-a",
  });

  const bindings = listBrowserRowPinBindings({
    mainSurfaceId: "main-1",
    surfaceId: "tab-2",
    secondaryTabs: [{ surfaceId: "tab-2" }, { surfaceId: "tab-3" }],
  });
  assert.equal(bindings.length, 2);
  const scopes = bindings.map((b) => b.scopeKey).sort();
  assert.deepEqual(scopes, ["channel:chan-a", "thread:t1"]);
});

test("parseBrowserRowPinLink accepts channel and thread deep links", async () => {
  const { parseBrowserRowPinLink } = await import(
    "./pinBrowserToConversation.ts"
  );

  assert.deepEqual(parseBrowserRowPinLink("/channels/chan-a"), {
    scopeKey: "channel:chan-a",
    channelId: "chan-a",
    threadRoot: null,
  });
  assert.deepEqual(
    parseBrowserRowPinLink("/channels/chan-a/posts/root-1"),
    {
      scopeKey: "thread:root-1",
      channelId: "chan-a",
      threadRoot: "root-1",
    },
  );
  assert.equal(parseBrowserRowPinLink("not-a-link"), null);
});
