/**
 * Regression for #7234: the Inbox conversation pane rendered relay system
 * events (kind 40099) as ordinary message bubbles, so a `dm_created` payload
 * showed up as raw JSON authored by a truncated hex pubkey. The channel
 * timeline routes the same event to `SystemMessageRow` and shows nothing for a
 * payload it has no copy for; the Inbox now matches.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
});

after(() => dom.window.close());

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AUTHOR = "a".repeat(64);
const RELAY = "f".repeat(64);

const CHANNEL = {
  id: CHANNEL_ID,
  name: "general",
  channelType: "stream",
  archivedAt: null,
};

function event(overrides) {
  return {
    id: overrides.id,
    kind: overrides.kind ?? 9,
    pubkey: overrides.pubkey ?? AUTHOR,
    content: overrides.content ?? "hello",
    created_at: overrides.created_at ?? 1_700_000_000,
    tags: overrides.tags ?? [["h", CHANNEL_ID]],
    sig: "0".repeat(128),
  };
}

const SYSTEM_EVENT = event({
  id: "system-1",
  kind: 40099,
  pubkey: RELAY,
  content: JSON.stringify({
    actor: AUTHOR,
    participants: [AUTHOR, "b".repeat(64)],
    type: "dm_created",
  }),
  created_at: 1_700_000_100,
});

async function renderContextMessages(events) {
  const { renderHook } = await import("@testing-library/react");
  const { useHomeInboxContextMessages } = await import(
    "./useHomeInboxContextMessages.ts"
  );

  const view = renderHook(() =>
    useHomeInboxContextMessages({
      currentPubkey: AUTHOR,
      events,
      profiles: {},
      reactionEvents: [],
      selectedChannel: CHANNEL,
      selectedEventId: "root-1",
      selectedItem: { id: "root-1", item: { pubkey: AUTHOR } },
    }),
  );
  return view.result.current;
}

test("inbox context drops relay system events", async () => {
  const messages = await renderContextMessages([
    event({ id: "root-1", content: "Please review the release checklist." }),
    SYSTEM_EVENT,
  ]);

  assert.equal(messages.length, 1, "only the real message survives");
  assert.equal(messages[0].id, "root-1");
  assert.ok(
    !messages.some((message) => message.body?.includes("dm_created")),
    "no raw system payload reaches the Inbox",
  );
  assert.ok(
    !messages.some((message) => message.rawPubkey === RELAY),
    "the relay signer is never surfaced as an Inbox author",
  );
});

test("inbox context keeps ordinary messages untouched", async () => {
  const messages = await renderContextMessages([
    event({ id: "root-1", content: "first" }),
    event({ id: "reply-1", content: "second", created_at: 1_700_000_200 }),
  ]);

  assert.deepEqual(
    messages.map((message) => message.id),
    ["root-1", "reply-1"],
  );
});
