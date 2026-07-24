import assert from "node:assert/strict";
import { test } from "node:test";

import { deriveServingIndicator } from "./servingUsage.ts";

function usage(overrides = {}) {
  return {
    inflight: 0,
    peakInflight: 0,
    requestsServed: 0,
    tokensServed: 0,
    tokensPerSecond: 0,
    localAttempts: 0,
    remoteAttempts: 0,
    endpointAttempts: 0,
    peers: 0,
    ...overrides,
  };
}

test("hidden when not sharing", () => {
  const i = deriveServingIndicator(usage({ requestsServed: 5 }), false);
  assert.equal(i.show, false);
});

test("hidden when usage not yet fetched", () => {
  const i = deriveServingIndicator(null, true);
  assert.equal(i.show, false);
});

test("sharing but nothing served yet -> idle, no detail", () => {
  const i = deriveServingIndicator(usage(), true);
  assert.equal(i.show, true);
  assert.equal(i.active, false);
  assert.equal(i.hasRemoteConsumers, false);
  assert.match(i.label, /Idle/);
});

test("only local agent traffic -> not a remote consumer", () => {
  const i = deriveServingIndicator(
    usage({ requestsServed: 4, localAttempts: 4, tokensPerSecond: 30 }),
    true,
  );
  assert.equal(i.hasRemoteConsumers, false);
  assert.match(i.label, /Idle/); // served earlier, none live now
  assert.match(i.detail, /4 requests served/);
});

test("local agent live now -> serving your agent", () => {
  const i = deriveServingIndicator(
    usage({ inflight: 1, localAttempts: 2, tokensPerSecond: 28 }),
    true,
  );
  assert.equal(i.active, true);
  assert.equal(i.hasRemoteConsumers, false);
  assert.match(i.label, /Serving your agent/);
  assert.match(i.label, /1 live/);
});

test("remote consumer, not live -> used by another member (headline case)", () => {
  const i = deriveServingIndicator(
    usage({
      requestsServed: 7,
      remoteAttempts: 6,
      endpointAttempts: 1,
      peers: 2,
    }),
    true,
  );
  assert.equal(i.hasRemoteConsumers, true);
  assert.equal(i.active, false);
  assert.match(i.label, /another member/);
  assert.match(i.label, /7 requests/); // remote+endpoint = 7
  assert.match(i.detail, /2 peers/);
});

test("remote consumer live now -> in use now, singular peer/request grammar", () => {
  const i = deriveServingIndicator(
    usage({ inflight: 1, remoteAttempts: 1, peers: 1, tokensPerSecond: 31 }),
    true,
  );
  assert.equal(i.active, true);
  assert.equal(i.hasRemoteConsumers, true);
  assert.match(i.label, /In use now by another member/);
  assert.match(i.detail, /1 peer\b/); // singular
});
