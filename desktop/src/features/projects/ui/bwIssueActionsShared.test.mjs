import assert from "node:assert/strict";
import test from "node:test";
import {
  bwReadyStreamOptions,
  bwReadyUpdateUnavailable,
} from "./bwIssueActionsShared.ts";

const repo = `30617:${"b".repeat(64)}:repo`;
const issueId = "a".repeat(64);

function updateRecord(id, previous) {
  const tags = [
    ["record", "issue-update"],
    ["a", repo],
    ["policy", "p".repeat(64)],
    ["issue", issueId],
  ];
  if (previous) tags.push(["previous", previous]);
  return {
    id,
    pubkey: "c".repeat(64),
    created_at: 1,
    kind: 46100,
    tags,
    content: "{}",
  };
}

function snapshot(records) {
  return {
    repo,
    activation: { policy: "p".repeat(64), genesis: "g".repeat(64) },
    projection: {
      issues: {},
      issue_fields: {},
      conflicts: [],
      children: {},
      relations: [],
      artifact_verdicts: {},
    },
    records: Object.fromEntries(records.map((r) => [r.id, r])),
    decisions: {},
    notices: {},
  };
}

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

test("an issue with no issue-update record yet has no update to send to ready", () => {
  assert.equal(bwReadyUpdateUnavailable(snapshot([]), issueId), true);
});

test("an issue with a single issue-update head has an update available", () => {
  const update = updateRecord("1".repeat(64));
  assert.equal(bwReadyUpdateUnavailable(snapshot([update]), issueId), false);
});

test("a forked issue-update chain has no resolvable head, so update stays unavailable", () => {
  const forkA = updateRecord("1".repeat(64));
  const forkB = updateRecord("2".repeat(64));
  assert.equal(
    bwReadyUpdateUnavailable(snapshot([forkA, forkB]), issueId),
    true,
  );
});
