import assert from "node:assert/strict";
import { test } from "node:test";

import {
  deriveRelayCloneUrl,
  effectiveCloneUrls,
  resolveRelayOrigin,
} from "./projectCloneUrl.ts";

const OWNER = "a".repeat(64);
const ORIGIN = "https://relay.example";

test("deriveRelayCloneUrl builds the canonical relay-hosted path", () => {
  assert.equal(
    deriveRelayCloneUrl(ORIGIN, OWNER, "flappy-bee"),
    `${ORIGIN}/git/${OWNER}/flappy-bee`,
  );
});

test("deriveRelayCloneUrl lowercases the owner pubkey", () => {
  const upper = "A".repeat(64);
  assert.equal(
    deriveRelayCloneUrl(ORIGIN, upper, "repo"),
    `${ORIGIN}/git/${OWNER}/repo`,
  );
});

test("deriveRelayCloneUrl tolerates a trailing slash on the origin", () => {
  assert.equal(
    deriveRelayCloneUrl(`${ORIGIN}/`, OWNER, "repo"),
    `${ORIGIN}/git/${OWNER}/repo`,
  );
});

test("deriveRelayCloneUrl fails closed on an unresolved origin", () => {
  assert.equal(deriveRelayCloneUrl(null, OWNER, "repo"), null);
  assert.equal(deriveRelayCloneUrl(undefined, OWNER, "repo"), null);
  assert.equal(deriveRelayCloneUrl("", OWNER, "repo"), null);
});

test("deriveRelayCloneUrl declines a non-hex or wrong-length owner", () => {
  assert.equal(deriveRelayCloneUrl(ORIGIN, "short", "repo"), null);
  assert.equal(deriveRelayCloneUrl(ORIGIN, "z".repeat(64), "repo"), null);
});

test("deriveRelayCloneUrl declines a missing repo id", () => {
  assert.equal(deriveRelayCloneUrl(ORIGIN, OWNER, ""), null);
});

test("effectiveCloneUrls honors explicit clone URLs over the derived default", () => {
  const explicit = ["https://github.com/octocat/hello"];
  assert.deepEqual(
    effectiveCloneUrls(explicit, ORIGIN, OWNER, "repo"),
    explicit,
  );
});

test("effectiveCloneUrls derives a default when none is advertised", () => {
  assert.deepEqual(effectiveCloneUrls([], ORIGIN, OWNER, "flappy-bee"), [
    `${ORIGIN}/git/${OWNER}/flappy-bee`,
  ]);
});

test("effectiveCloneUrls returns empty when no default can be derived", () => {
  assert.deepEqual(effectiveCloneUrls([], null, OWNER, "repo"), []);
});

test("resolveRelayOrigin uses the populated cache without fetching", async () => {
  let calls = 0;
  const origin = await resolveRelayOrigin(ORIGIN, async () => {
    calls += 1;
    return "https://wrong.example";
  });

  assert.equal(origin, ORIGIN);
  assert.equal(calls, 0);
});

test("resolveRelayOrigin fetches when startup cache is not ready", async () => {
  let calls = 0;
  const origin = await resolveRelayOrigin(null, async () => {
    calls += 1;
    return ORIGIN;
  });

  assert.equal(origin, ORIGIN);
  assert.equal(calls, 1);
});

test("resolveRelayOrigin fails open for explicitly cloned projects", async () => {
  const origin = await resolveRelayOrigin(null, async () => {
    throw new Error("IPC unavailable");
  });

  assert.equal(origin, null);
});
