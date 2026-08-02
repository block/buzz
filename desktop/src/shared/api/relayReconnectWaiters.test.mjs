import assert from "node:assert/strict";
import test from "node:test";

import { RelayReconnectWaiters } from "./relayReconnectWaiters.ts";

test("settle releases every operation waiting on a successful reconnect", async () => {
  const waiters = new RelayReconnectWaiters();
  const first = waiters.wait();
  const second = waiters.wait();

  waiters.settle();

  await Promise.all([first, second]);
});

test("settle rejects every operation after a failed reconnect", async () => {
  const waiters = new RelayReconnectWaiters();
  const first = waiters.wait();
  const second = waiters.wait();
  const error = new Error("relay unavailable");

  waiters.settle(error);

  await assert.rejects(first, error);
  await assert.rejects(second, error);
});
