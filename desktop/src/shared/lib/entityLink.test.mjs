import assert from "node:assert/strict";
import test from "node:test";

import {
  buildIssueLink,
  buildPullRequestLink,
  buildRepoLink,
  buildRepoTargetLink,
  entityLinkProjectRouteId,
  isEntityLink,
  parseEntityLink,
} from "./entityLink.ts";

const OWNER =
  "71d67180ba17e749ee825fc8819c9c6ee7003617e1c126504f9b658070ab9224";
const EVENT_ID =
  "c3b589fa5713ba25bad6dc095e2de00a4ac8f50050fdea00fc6444e603be1dd1";

// Golden format strings — must match the Rust builder in
// crates/buzz-cli/src/links.rs (`golden_format_matches_desktop` test).
test("builders emit the canonical cross-language link format", () => {
  assert.equal(
    buildPullRequestLink({ id: EVENT_ID, owner: OWNER, dtag: "buzz-world" }),
    `buzz://pr?id=${EVENT_ID}&owner=${OWNER}&d=buzz-world`,
  );
  assert.equal(
    buildIssueLink({ id: EVENT_ID, owner: OWNER, dtag: "buzz-world" }),
    `buzz://issue?id=${EVENT_ID}&owner=${OWNER}&d=buzz-world`,
  );
  assert.equal(
    buildRepoLink({ owner: OWNER, dtag: "buzz-world" }),
    `buzz://repo?owner=${OWNER}&d=buzz-world`,
  );
});

test("builders reject invalid identifiers", () => {
  assert.throws(() =>
    buildRepoLink({ owner: "not-a-pubkey", dtag: "buzz-world" }),
  );
  assert.throws(() => buildRepoLink({ owner: OWNER, dtag: ".hidden" }));
  assert.throws(() => buildRepoLink({ owner: OWNER, dtag: "a..b" }));
  assert.throws(() =>
    buildPullRequestLink({ id: "short", owner: OWNER, dtag: "buzz-world" }),
  );
});

test("parseEntityLink round-trips built links", () => {
  const link = buildPullRequestLink({
    id: EVENT_ID,
    owner: OWNER,
    dtag: "buzz-world",
  });
  assert.deepEqual(parseEntityLink(link), {
    ok: true,
    value: { type: "pr", id: EVENT_ID, owner: OWNER, dtag: "buzz-world" },
  });

  const repoLink = buildRepoLink({ owner: OWNER, dtag: "buzz-world" });
  assert.deepEqual(parseEntityLink(repoLink), {
    ok: true,
    value: { type: "repo", owner: OWNER, dtag: "buzz-world" },
  });
});

test("repository target builder preserves root identity and round-trips", () => {
  const href = buildRepoTargetLink({
    owner: OWNER.toUpperCase(),
    dtag: "buzz-world",
    ref: "feature/repo-links",
    path: "PLANS/Plan 1.md",
  });
  assert.equal(
    href,
    `buzz://repo?owner=${OWNER}&d=buzz-world&ref=feature%2Frepo-links&path=PLANS%2FPlan+1.md`,
  );
  assert.deepEqual(parseEntityLink(href), {
    ok: true,
    value: {
      type: "repo",
      owner: OWNER,
      dtag: "buzz-world",
      target: {
        ref: "feature/repo-links",
        refKind: "branch",
        path: "PLANS/Plan 1.md",
      },
    },
  });
  const parsed = parseEntityLink(href);
  assert.ok(parsed.ok);
  assert.equal(
    entityLinkProjectRouteId(parsed.value),
    `30617:${OWNER}:buzz-world`,
  );
  assert.equal(
    buildRepoLink({ owner: OWNER, dtag: "buzz-world" }),
    `buzz://repo?owner=${OWNER}&d=buzz-world`,
  );
});

test("repository target classifies full commits", () => {
  const commit = "B".repeat(40);
  const parsed = parseEntityLink(
    buildRepoTargetLink({
      owner: OWNER,
      dtag: "buzz-world",
      ref: commit,
      path: "README.md",
    }),
  );
  assert.ok(parsed.ok && parsed.value.type === "repo" && parsed.value.target);
  assert.deepEqual(parsed.value.target, {
    ref: commit.toLowerCase(),
    refKind: "commit",
    path: "README.md",
  });
});

test("repository target rejects incomplete, duplicate, old, and invalid coordinates", () => {
  const base = `buzz://repo?owner=${OWNER}&d=buzz-world`;
  const cases = [
    [`${base}&ref=main`, "incomplete-repo-target"],
    [`${base}&path=README.md`, "incomplete-repo-target"],
    [`${base}&ref=main&ref=dev&path=README.md`, "duplicate-param"],
    [`${base}&ref=main&path=README.md&path=OTHER.md`, "duplicate-param"],
    [
      `buzz://repo?owner=${OWNER}&repo=buzz&ref=main&path=README.md`,
      "unknown-param",
    ],
    [`${base}&ref=refs%2Ftags%2Fv1&path=README.md`, "invalid-ref"],
    [`${base}&ref=feature%2F..%2Fmain&path=README.md`, "invalid-ref"],
    [`${base}&ref=bad+ref&path=README.md`, "invalid-ref"],
    [`${base}&ref=main&path=%2Fetc%2Fpasswd`, "invalid-path"],
    [`${base}&ref=main&path=docs%2F..%2FREADME.md`, "invalid-path"],
    [`${base}&ref=main&path=docs%5CREADME.md`, "invalid-path"],
    [`${base}&ref=main&path=docs%2F%2FREADME.md`, "invalid-path"],
  ];
  for (const [href, reason] of cases) {
    assert.deepEqual(parseEntityLink(href), { ok: false, reason }, href);
  }
});

test("repository target rejects URL credentials and ports", () => {
  assert.deepEqual(
    parseEntityLink(`buzz://user@repo?owner=${OWNER}&d=buzz-world`),
    { ok: false, reason: "invalid-structure" },
  );
  assert.deepEqual(
    parseEntityLink(`buzz://repo:99?owner=${OWNER}&d=buzz-world`),
    { ok: false, reason: "invalid-structure" },
  );
});

test("repository target builder rejects invalid ref and path", () => {
  assert.throws(() =>
    buildRepoTargetLink({
      owner: OWNER,
      dtag: "buzz-world",
      ref: "bad ref",
      path: "README.md",
    }),
  );
  assert.throws(() =>
    buildRepoTargetLink({
      owner: OWNER,
      dtag: "buzz-world",
      ref: "main",
      path: "../README.md",
    }),
  );
});

test("parseEntityLink lowercase-normalizes hex identifiers", () => {
  const parsed = parseEntityLink(
    `buzz://issue?id=${EVENT_ID.toUpperCase()}&owner=${OWNER.toUpperCase()}&d=buzz-world`,
  );
  assert.deepEqual(parsed, {
    ok: true,
    value: { type: "issue", id: EVENT_ID, owner: OWNER, dtag: "buzz-world" },
  });
});

test("parseEntityLink rejects malformed links", () => {
  const cases = [
    ["not a url at all", "invalid-url"],
    [`https://pr?id=${EVENT_ID}&owner=${OWNER}&d=repo`, "wrong-scheme"],
    [`buzz://message?channel=x&id=${EVENT_ID}`, "wrong-host"],
    [`buzz://pr?id=${EVENT_ID}&owner=nope&d=repo`, "invalid-owner"],
    [`buzz://pr?id=${EVENT_ID}&owner=${OWNER}&d=.hidden`, "invalid-dtag"],
    [`buzz://pr?id=${EVENT_ID}&owner=${OWNER}`, "invalid-dtag"],
    [`buzz://pr?owner=${OWNER}&d=repo`, "invalid-id"],
    [`buzz://issue?id=short&owner=${OWNER}&d=repo`, "invalid-id"],
  ];
  for (const [href, reason] of cases) {
    assert.deepEqual(parseEntityLink(href), { ok: false, reason }, href);
  }
});

test("isEntityLink matches entity hosts and excludes message links", () => {
  assert.equal(isEntityLink(`buzz://pr?id=${EVENT_ID}`), true);
  assert.equal(isEntityLink(`buzz://issue?id=${EVENT_ID}`), true);
  assert.equal(isEntityLink(`buzz://repo?owner=${OWNER}`), true);
  assert.equal(isEntityLink("buzz://message?channel=x&id=y"), false);
  assert.equal(isEntityLink("https://github.com/block/buzz"), false);
  assert.equal(isEntityLink(null), false);
});

test("entityLinkProjectRouteId emits the canonical 30617 coordinate route id", () => {
  const parsed = parseEntityLink(
    buildRepoLink({ owner: OWNER, dtag: "buzz-world" }),
  );
  assert.ok(parsed.ok);
  assert.equal(
    entityLinkProjectRouteId(parsed.value),
    `30617:${OWNER}:buzz-world`,
  );
});

test("parseEntityLink rejects noncanonical extras", () => {
  // Unexpected path segments — reserved for future versioning.
  assert.deepEqual(
    parseEntityLink(
      `buzz://pr/ignored?id=${EVENT_ID}&owner=${OWNER}&d=buzz-world`,
    ),
    { ok: false, reason: "unexpected-path" },
  );
  // Fragment — not part of the canonical format.
  assert.deepEqual(
    parseEntityLink(`buzz://repo?owner=${OWNER}&d=buzz-world#section`),
    { ok: false, reason: "unexpected-fragment" },
  );
  // Unknown query parameter — reject to preserve forward-compat posture.
  assert.deepEqual(
    parseEntityLink(
      `buzz://repo?owner=${OWNER}&d=buzz-world&relay=wss%3A%2F%2Frelay.example`,
    ),
    { ok: false, reason: "unknown-param" },
  );
  // Duplicate required parameter — reject.
  assert.deepEqual(
    parseEntityLink(`buzz://repo?owner=${OWNER}&d=buzz-world&owner=${OWNER}`),
    { ok: false, reason: "duplicate-param" },
  );
});
