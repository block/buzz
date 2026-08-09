import assert from "node:assert/strict";
import test from "node:test";

import {
  buildFileLink,
  fileLinkBasename,
  isFileLink,
  parseFileLink,
} from "./fileLink.ts";

const PATH = "docs/guides/DEPLOY_RUNBOOK.md";

test("builder emits the canonical link format", () => {
  assert.equal(
    buildFileLink({ path: PATH }),
    `buzz://file?path=${encodeURIComponent(PATH)}`,
  );
  assert.equal(
    buildFileLink({ path: PATH, reveal: true }),
    `buzz://file?path=${encodeURIComponent(PATH)}&reveal=1`,
  );
  assert.equal(
    buildFileLink({ path: "buzz/README.md", root: "repos" }),
    "buzz://file?path=buzz%2FREADME.md&root=repos",
  );
});

test("builder omits root=nest so the format has one spelling", () => {
  assert.equal(
    buildFileLink({ path: PATH, root: "nest" }),
    buildFileLink({ path: PATH }),
  );
});

test("builder rejects paths that leave the root", () => {
  for (const bad of [
    "",
    "/etc/passwd",
    "../outside",
    "a/../../b",
    "C:/Windows",
    "a\\..\\b",
    "a//b",
    "a/\0/b",
  ]) {
    assert.throws(() => buildFileLink({ path: bad }), /fileLink/, bad);
  }
});

test("build → parse round-trips, including a path with spaces", () => {
  const spaced = "docs/design notes v2.pdf";
  const parsed = parseFileLink(buildFileLink({ path: spaced, reveal: true }));
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.value, {
    path: spaced,
    root: "nest",
    reveal: true,
  });
});

test("parse defaults root to nest and reveal to false", () => {
  const parsed = parseFileLink(`buzz://file?path=${PATH}`);
  assert.equal(parsed.ok, true);
  assert.deepEqual(parsed.value, { path: PATH, root: "nest", reveal: false });
});

test("parse accepts the repos root", () => {
  const parsed = parseFileLink("buzz://file?path=buzz/README.md&root=repos");
  assert.equal(parsed.ok, true);
  assert.equal(parsed.value.root, "repos");
});

test("parse rejects paths that escape the root", () => {
  // Percent-encoded traversal must be caught after decoding, not before.
  const encodedDotDot = parseFileLink("buzz://file?path=%2E%2E%2Fsecrets");
  assert.equal(encodedDotDot.ok, false);
  assert.equal(encodedDotDot.reason, "bad-path");

  for (const bad of [
    "buzz://file?path=/etc/passwd",
    "buzz://file?path=../../.ssh/id_rsa",
    "buzz://file?path=a/../../b",
  ]) {
    assert.equal(parseFileLink(bad).ok, false, bad);
  }
});

test("parse rejects malformed links rather than guessing", () => {
  const cases = [
    ["buzz://file", "missing-path"],
    ["buzz://file?path=", "bad-path"],
    ["buzz://message?channel=a&id=b", "wrong-host"],
    ["https://example.com/file?path=a", "wrong-scheme"],
    ["buzz://file/extra?path=a", "unexpected-path"],
    ["buzz://file?path=a#frag", "unexpected-fragment"],
    ["buzz://file?path=a&nope=1", "unknown-param"],
    ["buzz://file?path=a&path=b", "duplicate-param"],
    ["buzz://file?path=a&root=home", "unknown-root"],
    ["buzz://file?path=a&reveal=yes", "bad-reveal"],
    ["not a url", "invalid-url"],
  ];
  for (const [url, reason] of cases) {
    const result = parseFileLink(url);
    assert.equal(result.ok, false, url);
    assert.equal(result.reason, reason, url);
  }
});

test("isFileLink is a cheap prefix check that excludes other buzz links", () => {
  assert.equal(isFileLink(buildFileLink({ path: PATH })), true);
  assert.equal(isFileLink("buzz://message?channel=a&id=b"), false);
  assert.equal(isFileLink("buzz://pr?id=a&owner=b&d=c"), false);
  // No query string — cannot carry a path, so it is not a file link.
  assert.equal(isFileLink("buzz://file"), false);
  assert.equal(isFileLink(undefined), false);
  assert.equal(isFileLink(null), false);
});

test("basename is the display label", () => {
  assert.equal(
    fileLinkBasename({ path: PATH, root: "nest", reveal: false }),
    "DEPLOY_RUNBOOK.md",
  );
  assert.equal(
    fileLinkBasename({ path: "README.md", root: "nest", reveal: false }),
    "README.md",
  );
});
