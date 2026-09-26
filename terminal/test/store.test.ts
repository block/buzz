import assert from "node:assert/strict";
import test from "node:test";
import type {
  HostMessage,
  MessageRequest,
  RelayEvent,
  Requests,
} from "../src/protocol.ts";
import { HISTORY_LIMIT, Store } from "../src/store.ts";
import { HostError, type Transport } from "../src/transport.ts";

const me = "a".repeat(64),
  relay = "b".repeat(64),
  atlas = "c".repeat(64),
  nova = "d".repeat(64);
const first = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
  second = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
let sequence = 0;
const event = (
  kind: number,
  pubkey: string,
  tags: string[][],
  content = "",
  time = 1,
): RelayEvent => ({
  id: (++sequence).toString(16).padStart(64, "0"),
  kind,
  pubkey,
  tags,
  content,
  created_at: time,
  sig: "",
});
const receive = (
  store: Store,
  value: RelayEvent,
  subscriptionId = "discovery",
  ownerPubkey?: string,
) => store.handle({ type: "event", event: value, subscriptionId, ownerPubkey });

function fixture(): Store {
  const store = new Store();
  store.handle({
    type: "ready",
    protocolVersion: 1,
    pubkey: me,
    relayPubkey: relay,
    relayUrl: "wss://test.example",
  });
  store.handle({ type: "connection", status: "connected" });
  for (const channel of [first, second])
    receive(
      store,
      event(39002, relay, [
        ["d", channel],
        ["p", me],
        ["p", atlas],
        ["p", nova],
      ]),
    );
  receive(store, event(0, atlas, [], '{"name":"Twin"}'), "profiles", me);
  receive(store, event(0, nova, [], '{"name":"Twin"}'), "profiles", me);
  return store;
}

class Delivery implements Transport {
  requests: MessageRequest[] = [];
  resolve?: (result: unknown) => void;
  reject?: (error: Error) => void;
  start(_receive: (message: HostMessage) => void): void {}
  close(): void {}
  request<K extends keyof Requests>(
    _method: K,
    params: Requests[K],
  ): Promise<unknown> {
    this.requests.push(params as MessageRequest);
    return new Promise((resolve, reject) => {
      this.resolve = resolve;
      this.reject = reject;
    });
  }
  accept(): void {
    const request = this.requests.at(-1);
    assert.ok(request);
    this.resolve?.({
      accepted: true,
      event: event(9, me, [["h", request.channelId]], request.content),
    });
  }
}

test("late send acknowledgement cannot clear another context or a newer draft", async () => {
  const store = fixture(),
    delivery = new Delivery();
  const a = store.open(first);
  store.selectRecipient(a, atlas);
  a.draft = "review A";
  const sending = store.send(a, delivery);
  a.draft = "next question for A";
  const b = store.open(second);
  store.selectRecipient(b, nova);
  b.draft = "keep B";
  delivery.accept();
  await sending;
  assert.equal(a.draft, "next question for A");
  assert.equal(b.draft, "keep B");
  assert.equal(store.current, b);
  assert.equal(delivery.requests[0].channelId, first);
  assert.deepEqual(delivery.requests[0].recipientPubkeys, [atlas]);
  assert.equal(store.events(a)[0].content, "review A");
  assert.equal(store.events(b).length, 0);
});

test("uncertain delivery retries the same snapshot and blocks a recipient change", async () => {
  const store = fixture(),
    delivery = new Delivery(),
    view = store.open(first);
  store.selectRecipient(view, atlas);
  view.draft = "original";
  const sending = store.send(view, delivery);
  delivery.reject?.(new HostError("uncertain", "unknown"));
  await sending;
  assert.throws(() => store.selectRecipient(view, nova), /pending send/);
  view.draft = "newer text";
  const retrying = store.send(view, delivery, true);
  assert.deepEqual(delivery.requests[1], delivery.requests[0]);
  delivery.accept();
  await retrying;
  assert.equal(view.draft, "newer text");
  assert.equal(view.pending, undefined);
});

test("signed roster authority, newest membership, and loss block sends and purge history", async () => {
  const store = fixture(),
    view = store.open(first),
    delivery = new Delivery();
  store.selectRecipient(view, atlas);
  receive(store, event(9, atlas, [["h", first]], "private message"), view.key);
  receive(store, event(39002, nova, [["d", first]], "", 100));
  assert.equal(store.channels.get(first)?.joined, true);
  receive(
    store,
    event(
      39002,
      relay,
      [
        ["d", first],
        ["p", atlas],
      ],
      "",
      3,
    ),
  );
  receive(
    store,
    event(
      39002,
      relay,
      [
        ["d", first],
        ["p", me],
      ],
      "",
      2,
    ),
  );
  assert.equal(store.channels.get(first)?.joined, false);
  assert.equal(store.events(view).length, 0);
  view.draft = "must not send";
  await assert.rejects(store.send(view, delivery), /current channel member/);
  assert.equal(delivery.requests.length, 0);
});

test("overlapping replay deduplicates chat and never marks historical events unread", () => {
  const store = fixture(),
    a = store.open(first);
  store.open(second);
  const old = event(9, atlas, [["h", first]], "historical");
  receive(store, old, a.key);
  assert.equal(a.unread, 0);
  store.handle({ type: "eose", subscriptionId: a.key });
  receive(store, old, a.key);
  const live = event(9, atlas, [["h", first]], "live", 2);
  receive(store, live, a.key);
  receive(store, live, a.key);
  assert.equal(a.unread, 1);
  store.handle({ type: "connection", status: "disconnected" });
  store.handle({ type: "connection", status: "connected" });
  receive(store, old, a.key);
  receive(store, live, a.key);
  assert.equal(a.unread, 1);
  assert.equal(store.events(a).length, 2);
});

test("a busy sibling cannot evict an open thread's root or its bounded history", () => {
  const store = fixture(),
    channel = store.open(first);
  const root = event(9, atlas, [["h", first]], "root", 1);
  receive(store, root, channel.key);
  const thread = store.open(first, root.id);
  receive(store, root, thread.key);
  receive(
    store,
    event(
      9,
      atlas,
      [
        ["h", first],
        ["e", root.id, "", "reply"],
      ],
      "reply",
      2,
    ),
    thread.key,
  );
  for (let index = 0; index < HISTORY_LIMIT + 10; index++)
    receive(
      store,
      event(9, nova, [["h", first]], "sibling", 10 + index),
      channel.key,
    );
  assert.deepEqual(
    store.events(thread).map((item) => item.content),
    ["root", "reply"],
  );
  assert.equal(store.events(channel).length, HISTORY_LIMIT);
});

test("observer completion without sessionId ends its turn without reviving it", () => {
  const store = fixture();
  const send = (
    kind: string,
    seq: number,
    sessionId: string | null = "session-a",
  ) =>
    store.handle({
      type: "observer",
      agentPubkey: atlas,
      eventId: `observer-${seq}`,
      envelope: {
        kind,
        seq,
        sessionId,
        channelId: first,
        turnId: "turn-a",
        timestamp: new Date(seq * 1000).toISOString(),
        payload: {},
      },
    });
  send("turn_started", 1, null);
  send("acp_read", 2);
  assert.equal(store.activity.size, 1);
  send("turn_completed", 3, null);
  assert.equal([...store.activity.values()][0].state, "finished");
  assert.equal([...store.activity.values()][0].sessionId, "session-a");
  send("acp_read", 4);
  assert.equal([...store.activity.values()][0].state, "finished");
});

test("malformed or unowned observer payloads do not crash or populate activity", () => {
  const store = fixture();
  for (const envelope of [
    null,
    {},
    { kind: "batch", payload: { events: [null, {}] } },
  ]) {
    assert.doesNotThrow(() =>
      store.handle({
        type: "observer",
        eventId: `bad-${JSON.stringify(envelope)}`,
        agentPubkey: atlas,
        envelope,
      } as HostMessage),
    );
  }
  assert.equal(store.activity.size, 0);
});
