import assert from "node:assert/strict";
import test from "node:test";

import { managedAgentsForRelay } from "./managedAgentRelayScope.ts";

const agent = (name, relayUrl) => ({ name, pubkey: name, relayUrl });

const FIZZ_A = agent("fizz-a", "ws://relay-a:3000");
const FIZZ_B = agent("fizz-b", "ws://relay-b:3000");
const FIZZ_LOCAL = agent("fizz-local", "ws://localhost:3000");

test("keeps only the agents minted against the active relay", () => {
  assert.deepEqual(
    managedAgentsForRelay([FIZZ_A, FIZZ_B], "ws://relay-b:3000"),
    [FIZZ_B],
  );
});

test("matches canonically across localhost, port, and trailing slash", () => {
  assert.deepEqual(
    managedAgentsForRelay([FIZZ_LOCAL, FIZZ_B], "ws://127.0.0.1:3000/"),
    [FIZZ_LOCAL],
  );
});

test("an unknown or unparsable active relay leaves the list untouched", () => {
  // Better to show every agent than to show none because the caller could not
  // say which relay it is on.
  assert.deepEqual(managedAgentsForRelay([FIZZ_A, FIZZ_B], null), [
    FIZZ_A,
    FIZZ_B,
  ]);
  assert.deepEqual(managedAgentsForRelay([FIZZ_A, FIZZ_B], "not a url"), [
    FIZZ_A,
    FIZZ_B,
  ]);
});

test("a record with an unreadable relay url is kept", () => {
  const broken = agent("broken", "");
  assert.deepEqual(
    managedAgentsForRelay([broken, FIZZ_B], "ws://relay-a:3000"),
    [broken],
  );
});

test("an absent list is empty, not undefined", () => {
  assert.deepEqual(managedAgentsForRelay(undefined, "ws://relay-a:3000"), []);
});
