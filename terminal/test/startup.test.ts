import assert from "node:assert/strict";
import test from "node:test";
import { DemoTransport } from "../src/demo.ts";
import { parseLaunch } from "../src/launch.ts";
import type { HostMessage, Requests } from "../src/protocol.ts";
import { Session } from "../src/session.ts";
import { Store } from "../src/store.ts";

class Wire extends DemoTransport {
  calls: { method: keyof Requests; params: Requests[keyof Requests] }[] = [];
  notify: (message: HostMessage) => void = () => {};
  failCreate = false;
  override start(receive: (message: HostMessage) => void): void {
    this.notify = receive;
    super.start(receive);
  }
  override async request<K extends keyof Requests>(
    method: K,
    params: Requests[K],
  ): Promise<unknown> {
    this.calls.push({ method, params });
    if (method === "createChannel" && this.failCreate)
      throw new Error("uncertain create");
    return super.request(method, params);
  }
  writes() {
    return this.calls.filter(
      (call) => !["subscribe", "unsubscribe"].includes(call.method),
    );
  }
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));
const atlas = "3".repeat(64);
const engineering = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

test("each bare launch gets a fresh ID, but join requires exactly an existing-channel UUID", () => {
  assert.notEqual(parseLaunch([]).channelId, parseLaunch([]).channelId);
  assert.deepEqual(parseLaunch(["join", engineering.toUpperCase()]), {
    mode: "join",
    channelId: engineering,
    agent: undefined,
  });
  for (const args of [
    ["join"],
    ["join", "typo"],
    ["join", engineering, "extra"],
    ["typo"],
  ])
    assert.throws(() => parseLaunch(args), /Usage/);
  assert.throws(() => parseLaunch([], "Atlas"), /BUZZ_AGENT_PUBKEY/);
});

test("new launch creates once, attaches the exact agent, opens the composer, and reconnect never creates again", async () => {
  const launch = parseLaunch([], atlas),
    store = new Store(),
    wire = new Wire();
  const session = new Session(store, wire, launch);
  try {
    session.start();
    await settle();
    assert.equal(store.current?.channelId, launch.channelId);
    assert.equal(store.current?.recipient, atlas);
    assert.deepEqual(
      wire.writes().map((call) => call.method),
      ["createChannel", "addMember"],
    );
    const view = store.current;
    assert.ok(view);
    view.draft = "Keep my launch draft";
    store.open(engineering);
    wire.notify({ type: "connection", status: "disconnected" });
    wire.notify({ type: "connection", status: "connected" });
    await settle();
    assert.equal(store.current?.channelId, engineering);
    assert.equal(view.draft, "Keep my launch draft");
    assert.deepEqual(
      wire.writes().map((call) => call.method),
      ["createChannel", "addMember"],
    );
    const roster = store.channels.get(launch.channelId)?.roster;
    assert.ok(roster);
    wire.notify({
      type: "event",
      subscriptionId: "startup",
      event: {
        ...roster,
        id: "e".repeat(64),
        created_at: roster.created_at + 1,
        tags: roster.tags.filter((tag) => tag[0] !== "p" || tag[1] !== atlas),
      },
    });
    assert.equal(view.recipient, undefined);
    wire.notify({
      type: "event",
      subscriptionId: "startup",
      event: {
        ...roster,
        id: "f".repeat(64),
        created_at: roster.created_at + 2,
      },
    });
    assert.equal(
      view.recipient,
      undefined,
      "membership re-addition must not bypass recipient review",
    );
  } finally {
    session.close();
  }
});

test("joining an existing private membership opens history without any create, join, or invite writes", async () => {
  const store = new Store(),
    wire = new Wire();
  const session = new Session(store, wire, parseLaunch(["join", engineering]));
  try {
    session.start();
    await settle();
    assert.equal(store.current?.channelId, engineering);
    assert.equal(store.current?.recipient, atlas);
    assert.ok(
      store
        .events(store.current)
        .some((event) => event.content.includes("Review the relay")),
    );
    assert.deepEqual(wire.writes(), []);
  } finally {
    session.close();
  }
});

test("missing or forbidden join never falls back to creation, including an explicit retry", async () => {
  const store = new Store(),
    wire = new Wire();
  const launch = parseLaunch(["join", "cccccccc-cccc-4ccc-8ccc-cccccccccccc"]);
  const session = new Session(store, wire, launch);
  try {
    session.start();
    await settle();
    assert.equal(store.current, undefined);
    assert.match(store.notice, /Channel not found/);
    assert.equal(wire.writes().length, 1);
    assert.equal(wire.writes()[0].method, "joinChannel");
    assert.ok(session.startup);
    await assert.rejects(session.startup.retry(), /Channel not found/);
    assert.deepEqual(wire.writes()[1], wire.writes()[0]);
  } finally {
    session.close();
  }
});

test("uncertain creation retains channel and request identity for manual retry", async () => {
  const store = new Store(),
    wire = new Wire(),
    launch = parseLaunch([], atlas);
  wire.failCreate = true;
  const session = new Session(store, wire, launch);
  try {
    session.start();
    await settle();
    assert.match(store.notice, /uncertain create/);
    wire.notify({ type: "connection", status: "disconnected" });
    wire.notify({ type: "connection", status: "connected" });
    await settle();
    assert.equal(wire.writes().length, 1);
    wire.failCreate = false;
    assert.ok(session.startup);
    await session.startup.retry();
    await settle();
    assert.deepEqual(wire.writes()[1], wire.writes()[0]);
    assert.equal(store.current?.channelId, launch.channelId);
    assert.equal(store.current?.recipient, atlas);
  } finally {
    session.close();
  }
});

test("multiple owned agents default to the first agent without opening a picker", async () => {
  const store = new Store(),
    wire = new Wire(),
    launch = parseLaunch([]);
  const session = new Session(store, wire, launch);
  try {
    session.start();
    await settle();
    const view = store.current;
    assert.ok(view);
    assert.equal(view.recipient, atlas);
    assert.deepEqual(
      wire.writes().map((call) => call.method),
      ["createChannel", "addMember"],
    );
    assert.equal(
      store.preferredAgent,
      undefined,
      "automatic defaults are not manual choices",
    );
  } finally {
    session.close();
  }
});
