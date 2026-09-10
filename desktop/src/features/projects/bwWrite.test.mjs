import assert from "node:assert/strict";
import test from "node:test";
import {
  BwConflictError,
  submitBwIssueTextUpdate,
  submitBwTriageAction,
} from "./bwWrite.ts";

const repo = `30617:${"b".repeat(64)}:repo`;
const issueId = "a".repeat(64);

function record(id, { record: recordType, issue = issueId, previous }) {
  const tags = [
    ["record", recordType],
    ["a", repo],
    ["policy", "p".repeat(64)],
    ["issue", issue],
  ];
  if (previous) tags.push(["previous", previous]);
  return {
    id,
    pubkey: "c".repeat(64),
    created_at: 1,
    kind: 46100,
    tags,
    content:
      recordType === "triage-action"
        ? '{"action":"need-info","question":"q","recipient":"' +
          "d".repeat(64) +
          '"}'
        : "{}",
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

test("a forked issue-update chain refuses the text update up front without calling out", async () => {
  const forkA = record("1".repeat(64), { record: "issue-update" });
  const forkB = record("2".repeat(64), { record: "issue-update" });
  await assert.rejects(
    submitBwIssueTextUpdate({
      issueId,
      patch: { title: "New title" },
      repo,
      snapshot: snapshot([forkA, forkB]),
    }),
    BwConflictError,
  );
});

test("a forked triage-action chain refuses the triage action up front", async () => {
  const forkA = record("1".repeat(64), { record: "triage-action" });
  const forkB = record("2".repeat(64), { record: "triage-action" });
  await assert.rejects(
    submitBwTriageAction({
      fields: { action: "accept" },
      issueId,
      repo,
      snapshot: snapshot([forkA, forkB]),
    }),
    BwConflictError,
  );
});
