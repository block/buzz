import assert from "node:assert/strict";
import test from "node:test";
import { RelayChannelAccessRevocations } from "./relayChannelAccessRevocations.ts";

const revoked = "restricted: channel access revoked";
const sub = (channel) => ({ mode: "live", filter: { "#h": [channel] } });

test("a scoped huddle consumer is not notified when another channel is revoked", () => {
  const hints = new RelayChannelAccessRevocations();
  let huddle = 0;
  let unscoped = 0;
  const dispose = hints.subscribe(() => huddle++, "huddle");
  hints.subscribe(() => unscoped++);
  hints.notify(sub("other"), revoked);
  assert.equal(huddle, 0);
  assert.equal(unscoped, 1);
  hints.notify(sub("huddle"), revoked);
  assert.equal(huddle, 1);
  assert.equal(unscoped, 2);
  dispose();
  hints.notify(sub("huddle"), revoked);
  assert.equal(huddle, 1);
});

test("scope registrations have independent cleanup and ignore non-revocation messages", () => {
  const hints = new RelayChannelAccessRevocations();
  let calls = 0;
  const callback = () => calls++;
  const dispose = hints.subscribe(callback, "a");
  hints.subscribe(callback, "b");
  dispose();
  hints.notify(sub("b"), "rate-limited: retry later");
  assert.equal(calls, 0);
  hints.notify(sub("a"), revoked);
  assert.equal(calls, 0);
  hints.notify(sub("b"), revoked);
  assert.equal(calls, 1);
  hints.clear();
  hints.notify(sub("b"), revoked);
  assert.equal(calls, 1);
});
