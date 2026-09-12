import assert from "node:assert/strict";
import test from "node:test";
import { bwReadyStreamOptions } from "./bwIssueActionsShared.ts";

test("branches from the repo state become deduplicated stream options", () => {
  const { options, unavailable } = bwReadyStreamOptions(
    [{ name: "main" }, { name: "feature/x" }, { name: "main" }],
    false,
  );

  assert.deepEqual(options, ["main", "feature/x"]);
  assert.equal(unavailable, false);
});

test("a single-branch repo offers exactly one option", () => {
  const { options, unavailable } = bwReadyStreamOptions(
    [{ name: "main" }],
    false,
  );

  assert.deepEqual(options, ["main"]);
  assert.equal(unavailable, false);
});

test("an empty branch list after loading is unavailable, not a free-text escape", () => {
  const { options, unavailable } = bwReadyStreamOptions([], false);

  assert.deepEqual(options, []);
  assert.equal(unavailable, true);
});

test("an undefined branch list (query error or no data yet) is unavailable once loading finishes", () => {
  const { options, unavailable } = bwReadyStreamOptions(undefined, false);

  assert.deepEqual(options, []);
  assert.equal(unavailable, true);
});

test("still loading is never reported as unavailable, even with no branches yet", () => {
  const { options, unavailable } = bwReadyStreamOptions(undefined, true);

  assert.deepEqual(options, []);
  assert.equal(unavailable, false);
});
