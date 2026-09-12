import assert from "node:assert/strict";
import test from "node:test";
import {
  BwConflictError,
  submitBwIssueTextUpdate,
  submitBwReadyTransition,
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

function withMockInvoke(handler) {
  globalThis.window = { __TAURI_INTERNALS__: { invoke: handler } };
}

test("a matching delegation submits delegate:true and skips the previous tag (P4F)", async () => {
  const existingHead = record("1".repeat(64), { record: "triage-action" });
  let captured;
  withMockInvoke(async (_cmd, args) => {
    captured = args;
    return { eventId: "z".repeat(64), projection: {} };
  });
  await submitBwTriageAction({
    delegate: true,
    fields: { action: "accept" },
    issueId,
    repo,
    snapshot: snapshot([existingHead]),
  });
  assert.equal(captured.input.delegate, true);
  assert.equal(
    captured.input.tags.some((tag) => tag[0] === "previous"),
    false,
  );
});

test("no delegation keeps delegate:false and the ordinary previous chain (P4F)", async () => {
  const existingHead = record("1".repeat(64), { record: "triage-action" });
  let captured;
  withMockInvoke(async (_cmd, args) => {
    captured = args;
    return { eventId: "z".repeat(64), projection: {} };
  });
  await submitBwTriageAction({
    fields: { action: "accept" },
    issueId,
    repo,
    snapshot: snapshot([existingHead]),
  });
  assert.equal(captured.input.delegate, false);
  assert.deepEqual(
    captured.input.tags.find((tag) => tag[0] === "previous"),
    ["previous", existingHead.id],
  );
});

test("a ready transition whose real chain head is still triage auto-chains the missing backlog record first, then readies off it", async () => {
  const triageStateId = "3".repeat(64);
  const acceptAction = record("4".repeat(64), { record: "triage-action" });
  acceptAction.content = '{"action":"accept"}';
  const snap = snapshot([acceptAction]);
  snap.projection.issue_state = { [issueId]: { state: "triage" } };
  snap.projection.issue_state_id = { [issueId]: triageStateId };

  const calls = [];
  withMockInvoke(async (_cmd, args) => {
    calls.push(args);
    return { eventId: `${calls.length}`.repeat(64), projection: {} };
  });

  await submitBwReadyTransition({
    issueId,
    repo,
    snapshot: snap,
    stream: "windows",
  });

  assert.equal(calls.length, 2);
  assert.equal(calls[0].input.record, "issue-state");
  assert.deepEqual(calls[0].input.content, {
    state: "backlog",
    triage: acceptAction.id,
  });
  assert.deepEqual(
    calls[0].input.tags.find((tag) => tag[0] === "previous"),
    ["previous", triageStateId],
  );
  assert.equal(calls[1].input.content.state, "ready");
  assert.deepEqual(
    calls[1].input.tags.find((tag) => tag[0] === "previous"),
    ["previous", "1".repeat(64)],
  );
});
