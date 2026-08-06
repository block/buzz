import assert from "node:assert/strict";
import test from "node:test";

import {
  normalizeRepositoryPath,
  parseRepositoryRef,
} from "./repositoryTarget.ts";

test("parseRepositoryRef classifies commits and normalizes head refs", () => {
  assert.deepEqual(parseRepositoryRef("A".repeat(40)), {
    kind: "commit",
    value: "a".repeat(40),
  });
  assert.deepEqual(parseRepositoryRef("B".repeat(64)), {
    kind: "commit",
    value: "b".repeat(64),
  });
  assert.deepEqual(parseRepositoryRef("refs/heads/feature/repo-links"), {
    kind: "branch",
    value: "feature/repo-links",
  });
});

test("parseRepositoryRef rejects ambiguous and non-head refs", () => {
  for (const value of [
    "",
    "-main",
    "/main",
    "main/",
    "main.",
    "main.lock",
    "feature/../main",
    "feature//main",
    ".hidden/main",
    "feature/.hidden",
    "bad ref",
    "refs/tags/v1",
    "refs/nostr/main",
  ]) {
    assert.equal(parseRepositoryRef(value), null, value);
  }
});

test("normalizeRepositoryPath accepts relative Unicode paths", () => {
  assert.equal(
    normalizeRepositoryPath("GUIDES/café setup.md"),
    "GUIDES/café setup.md",
  );
});

test("normalizeRepositoryPath rejects absolute, traversal, ambiguous, control, and oversized paths", () => {
  for (const value of [
    "",
    "/etc/passwd",
    "../README.md",
    "docs/../README.md",
    "docs//README.md",
    "docs/./README.md",
    "docs\\README.md",
    "docs/README.md/",
    `docs/${String.fromCharCode(0)}README.md`,
    "é".repeat(2049),
  ]) {
    assert.equal(normalizeRepositoryPath(value), null, JSON.stringify(value));
  }
});
