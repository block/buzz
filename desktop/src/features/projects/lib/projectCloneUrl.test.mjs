import assert from "node:assert/strict";
import { test } from "node:test";

import { deriveRelayCloneUrl, effectiveCloneUrls } from "./projectCloneUrl.ts";

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
