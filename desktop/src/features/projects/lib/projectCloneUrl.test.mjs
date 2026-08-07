import assert from "node:assert/strict";
import { test } from "node:test";

import { deriveRelayCloneUrl, effectiveCloneUrls } from "./projectCloneUrl.ts";
import {
  projectRepoHost,
  projectRepoHostForProject,
} from "./projectRepoHost.ts";

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

test("projectRepoHost recognizes a canonical repository on the relay", () => {
  assert.deepEqual(projectRepoHost(`${ORIGIN}/git/${OWNER}/buzz`, ORIGIN), {
    kind: "buzz",
  });
});

test("projectRepoHost identifies an external repository by host", () => {
  assert.deepEqual(
    projectRepoHost("https://github.com/block/buzz.git", ORIGIN),
    { kind: "external", host: "github.com" },
  );
});

test("projectRepoHost treats a non-repository relay path as external", () => {
  assert.deepEqual(projectRepoHost(`${ORIGIN}/other/path`, ORIGIN), {
    kind: "external",
    host: "relay.example",
  });
});

test("projectRepoHost fails closed while either URL is unresolved", () => {
  assert.deepEqual(projectRepoHost(null, ORIGIN), { kind: "unresolved" });
  assert.deepEqual(projectRepoHost(`${ORIGIN}/git/${OWNER}/buzz`, null), {
    kind: "unresolved",
  });
  assert.deepEqual(projectRepoHost("not a URL", ORIGIN), {
    kind: "unresolved",
  });
});

test("projectRepoHostForProject recognizes an implicit relay repository", () => {
  assert.deepEqual(
    projectRepoHostForProject(
      { cloneUrls: [], dtag: "buzz", owner: OWNER },
      ORIGIN,
    ),
    { kind: "buzz" },
  );
});
