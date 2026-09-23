import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

const card = {
  hula: "playground",
  v: 1,
  name: "Demo",
  url: "https://app.example.com",
  pin: "4455",
  sid: "demo-1",
  stack: "hula-app",
};

afterEach(async () => {
  const { resetConversationPlaygroundPins } = await import(
    "./conversationPins.ts"
  );
  resetConversationPlaygroundPins();
});

test("pins are scoped to channel vs thread and stay in memory only", async () => {
  const {
    hasConversationPlaygroundPin,
    listConversationPlaygroundPins,
    pinPlaygroundToConversation,
  } = await import("./conversationPins.ts");

  pinPlaygroundToConversation("channel:chan-a", card);
  pinPlaygroundToConversation("thread:root-1", {
    ...card,
    sid: "demo-2",
    name: "Thread Demo",
  });

  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 1);
  assert.equal(
    listConversationPlaygroundPins("channel:chan-a")[0]?.sid,
    "demo-1",
  );
  assert.equal(listConversationPlaygroundPins("thread:root-1").length, 1);
  assert.equal(
    listConversationPlaygroundPins("thread:root-1")[0]?.name,
    "Thread Demo",
  );
  assert.equal(hasConversationPlaygroundPin("channel:chan-a", "demo-2"), false);
  assert.equal(listConversationPlaygroundPins("channel:chan-b").length, 0);
});

test("unpin removes the pin; pin is idempotent for the same sid", async () => {
  const {
    listConversationPlaygroundPins,
    pinPlaygroundToConversation,
    unpinPlaygroundFromConversation,
  } = await import("./conversationPins.ts");

  pinPlaygroundToConversation("channel:chan-a", card);
  pinPlaygroundToConversation("channel:chan-a", { ...card, name: "Renamed" });
  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 1);
  assert.equal(
    listConversationPlaygroundPins("channel:chan-a")[0]?.name,
    "Renamed",
  );

  unpinPlaygroundFromConversation("channel:chan-a", "demo-1");
  assert.equal(listConversationPlaygroundPins("channel:chan-a").length, 0);
});

test("requestOpenConversationPlaygroundPinsMenu bumps a per-scope nonce", async () => {
  const {
    getConversationPlaygroundPinsMenuOpenRequest,
    requestOpenConversationPlaygroundPinsMenu,
  } = await import("./conversationPins.ts");

  requestOpenConversationPlaygroundPinsMenu("channel:chan-a");
  const first = getConversationPlaygroundPinsMenuOpenRequest();
  assert.equal(first?.scopeKey, "channel:chan-a");
  requestOpenConversationPlaygroundPinsMenu("channel:chan-a");
  const second = getConversationPlaygroundPinsMenuOpenRequest();
  assert.equal(second?.scopeKey, "channel:chan-a");
  assert.notEqual(first?.nonce, second?.nonce);
});

test("listConversationPlaygroundPins returns stable refs across alternating scopes", async () => {
  // Regression: channel header + thread panel both mount ConversationPlaygroundPinsMenu.
  // A single global snapshot cache thrashing between scopes made useSyncExternalStore
  // see a new array every getSnapshot → Maximum update depth exceeded.
  const { listConversationPlaygroundPins, pinPlaygroundToConversation } =
    await import("./conversationPins.ts");

  pinPlaygroundToConversation("channel:chan-a", card);

  const channelA = listConversationPlaygroundPins("channel:chan-a");
  const threadEmpty = listConversationPlaygroundPins("thread:root-1");
  const channelB = listConversationPlaygroundPins("channel:chan-a");
  const threadEmptyAgain = listConversationPlaygroundPins("thread:root-1");
  const otherEmpty = listConversationPlaygroundPins("channel:chan-b");

  assert.equal(channelA, channelB);
  assert.equal(threadEmpty, threadEmptyAgain);
  assert.equal(threadEmpty, otherEmpty);
  assert.equal(channelA.length, 1);
  assert.equal(threadEmpty.length, 0);
});

test("conversationPlaygroundPinWebviewId is sanitize_pin_id-safe (no colon)", async () => {
  const { conversationPlaygroundPinWebviewId } = await import(
    "./conversationPins.ts"
  );
  const id = conversationPlaygroundPinWebviewId("demo-1");
  assert.equal(id, "playground-pin-demo-1");
  assert.match(id, /^[A-Za-z0-9_-]+$/);
  assert.ok(!id.includes(":"));
});

test("seedConversationPlaygroundPins hydrates a companion scope", async () => {
  const { listConversationPlaygroundPins, seedConversationPlaygroundPins } =
    await import("./conversationPins.ts");

  seedConversationPlaygroundPins("thread:root-1", [
    {
      sid: "demo-1",
      name: "Demo",
      url: "https://app.example.com",
      pin: "4455",
      stack: "hula-app",
    },
  ]);
  assert.equal(listConversationPlaygroundPins("thread:root-1").length, 1);
  assert.equal(
    listConversationPlaygroundPins("thread:root-1")[0]?.name,
    "Demo",
  );
  // Idempotent refresh
  seedConversationPlaygroundPins("thread:root-1", [
    {
      sid: "demo-1",
      name: "Renamed",
      url: "https://app.example.com",
    },
    {
      sid: "demo-2",
      name: "Second",
      url: "https://other.example.com",
    },
  ]);
  assert.equal(listConversationPlaygroundPins("thread:root-1").length, 2);
  assert.equal(
    listConversationPlaygroundPins("thread:root-1").find(
      (p) => p.sid === "demo-1",
    )?.name,
    "Renamed",
  );
});
