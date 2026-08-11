import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { RelayAuthChallengeBuffer } from "./relayAuthChallengeBuffer.ts";

test("early AUTH challenge is consumed once by the matching connection generation", () => {
  const buffer = new RelayAuthChallengeBuffer();
  buffer.defer("relay-challenge", 7);

  assert.equal(buffer.take(7), "relay-challenge");
  assert.equal(buffer.take(7), null);
});

test("early AUTH challenge from a stale generation is discarded", () => {
  const buffer = new RelayAuthChallengeBuffer();
  buffer.defer("stale-challenge", 3);

  assert.equal(buffer.take(4), null);
});

test("AUTH preparation defers until the connection is ready", async () => {
  const buffer = new RelayAuthChallengeBuffer();
  let created = false;

  const event = await buffer.prepare(
    "early-challenge",
    5,
    () => 5,
    () => false,
    async () => {
      created = true;
      return { id: "auth-event" };
    },
  );

  assert.equal(event, null);
  assert.equal(created, false);
  assert.equal(buffer.take(5), "early-challenge");
});

test("early AUTH challenge is signed once after the connection becomes ready", async () => {
  const buffer = new RelayAuthChallengeBuffer();
  const generation = 6;
  let ready = false;
  let createCount = 0;

  const earlyEvent = await buffer.prepare(
    "early-challenge",
    generation,
    () => generation,
    () => ready,
    async () => {
      createCount += 1;
      return { id: "auth-event" };
    },
  );
  ready = true;
  const challenge = buffer.take(generation);
  assert.equal(challenge, "early-challenge");

  const event = await buffer.prepare(
    challenge,
    generation,
    () => generation,
    () => ready,
    async () => {
      createCount += 1;
      return { id: "auth-event" };
    },
  );

  assert.equal(earlyEvent, null);
  assert.deepEqual(event, { id: "auth-event" });
  assert.equal(createCount, 1);
  assert.equal(buffer.take(generation), null);
});

test("AUTH preparation rechecks generation after async signing", async () => {
  const buffer = new RelayAuthChallengeBuffer();
  let generation = 8;

  const event = await buffer.prepare(
    "challenge",
    generation,
    () => generation,
    () => true,
    async () => {
      generation += 1;
      return { id: "stale-auth-event" };
    },
  );

  assert.equal(event, null);
});

test("both relay clients buffer AUTH until the native connection id is ready", () => {
  const testDir = path.dirname(fileURLToPath(import.meta.url));
  const sources = ["relayClientSession.ts", "readOnlyRelayClient.ts"].map(
    (file) => readFileSync(path.join(testDir, file), "utf8"),
  );

  for (const source of sources) {
    assert.match(source, /new RelayAuthChallengeBuffer\(\)/);
    assert.match(source, /earlyAuthChallenge\.take\(generation\)/);
    assert.match(source, /earlyAuthChallenge\.prepare\(/);
  }
});
