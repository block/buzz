import assert from "node:assert/strict";
import test from "node:test";
import type { HostMessage, Requests } from "../src/protocol.ts";
import { Session } from "../src/session.ts";
import { Store } from "../src/store.ts";
import type { Transport } from "../src/transport.ts";

class Wire implements Transport {
  receive: (message: HostMessage) => void = () => {};
  calls: { method: keyof Requests; params: Requests[keyof Requests] }[] = [];
  start(receive: (message: HostMessage) => void): void {
    this.receive = receive;
  }
  async request<K extends keyof Requests>(
    method: K,
    params: Requests[K],
  ): Promise<unknown> {
    this.calls.push({ method, params });
    return {};
  }
  close(): void {}
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

test("reconnect cleans up a view closed offline; profile filters remain within host bounds", async () => {
  const store = new Store(),
    wire = new Wire(),
    session = new Session(store, wire);
  session.start();
  const human = "a".repeat(64),
    relay = "b".repeat(64),
    channel = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
  wire.receive({
    type: "ready",
    protocolVersion: 1,
    pubkey: human,
    relayPubkey: relay,
    relayUrl: "wss://example.test",
  });
  wire.receive({ type: "connection", status: "connected" });
  wire.receive({
    type: "event",
    subscriptionId: "discovery",
    event: {
      id: "c".repeat(64),
      pubkey: relay,
      created_at: 1,
      kind: 39002,
      content: "",
      sig: "",
      tags: [
        ["d", channel],
        ["p", human],
        ...Array.from({ length: 749 }, (_, index) => [
          "p",
          index.toString(16).padStart(64, "0"),
        ]),
      ],
    },
  });
  const view = store.open(channel);
  await settle();
  const calls = wire.calls
    .filter((call) => call.method === "subscribe")
    .map((call) => call.params as Requests["subscribe"]);
  assert.ok(calls.some((call) => call.subscriptionId === view.key));
  assert.ok(
    calls.some(
      (call) =>
        call.subscriptionId === "channels" &&
        call.filters[0]["#d"]?.includes(channel) &&
        !call.filters[0]["#p"],
    ),
  );
  const profile = calls.find((call) => call.subscriptionId === "profiles");
  assert.ok(profile);
  assert.equal(
    profile.filters.flatMap((filter) => filter.authors ?? []).length,
    750,
  );
  assert.ok(
    profile.filters.every(
      (filter) => Buffer.byteLength(JSON.stringify(filter)) <= 16 * 1024,
    ),
  );
  wire.receive({ type: "connection", status: "disconnected" });
  store.views.delete(view.key);
  store.changed();
  await settle();
  wire.receive({ type: "connection", status: "connected" });
  await settle();
  assert.ok(
    wire.calls.some(
      (call) =>
        call.method === "unsubscribe" &&
        (call.params as Requests["unsubscribe"]).subscriptionId === view.key,
    ),
  );
  session.close();
});
