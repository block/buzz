import assert from "node:assert/strict";
import test from "node:test";
import {
  BwConflictError,
  submitBwInDevelopmentTransition,
  submitBwIssueTextUpdate,
  submitBwReadyTransition,
  submitBwTriageAction,
  submitBwWriterSelection,
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
      fields: { action: "need-info", question: "q", recipient: "d".repeat(64) },
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

test("a matching delegation is passed to the complete accept operation", async () => {
  let captured;
  withMockInvoke(async (command, args) => {
    captured = { command, args };
    return { eventId: "z".repeat(64), projection: {} };
  });
  await submitBwTriageAction({
    delegate: true,
    fields: { action: "accept" },
    issueId,
    repo,
  });
  assert.equal(captured.command, "accept_project_bw_issue");
  assert.deepEqual(captured.args.input, { delegated: true, issueId, repo });
});

test("ordinary accept delegates the whole chain without delegation", async () => {
  let captured;
  withMockInvoke(async (command, args) => {
    captured = { command, args };
    return { eventId: "z".repeat(64), projection: {} };
  });
  await submitBwTriageAction({
    fields: { action: "accept" },
    issueId,
    repo,
  });
  assert.equal(captured.command, "accept_project_bw_issue");
  assert.deepEqual(captured.args.input, { delegated: false, issueId, repo });
});

test("in-development delegates the lifecycle operation to Tauri", async () => {
  let captured;
  withMockInvoke(async (command, args) => {
    captured = { command, args };
    return { eventId: "3".repeat(64), projection: {} };
  });

  await submitBwInDevelopmentTransition({
    issueId,
    repo,
  });

  assert.equal(captured.command, "start_project_bw_development");
  assert.deepEqual(captured.args.input, { issueId, repo });
});

test("changing a ready writer delegates the resumable chain to Tauri", async () => {
  const newWriter = "d".repeat(64);
  let captured;
  withMockInvoke(async (command, args) => {
    captured = { command, args };
    return { eventId: "6".repeat(64), projection: {} };
  });

  await submitBwWriterSelection({
    delegate: newWriter,
    issueId,
    repo,
    snapshot: snapshot([]),
  });

  assert.equal(captured.command, "assign_project_bw_writer");
  assert.deepEqual(captured.args.input, {
    issueId,
    repo,
    writer: newWriter,
  });
});

test("ready delegates acceptance repair and transition to Tauri", async () => {
  let captured;
  withMockInvoke(async (command, args) => {
    captured = { command, args };
    return { eventId: "6".repeat(64), projection: {} };
  });

  await submitBwReadyTransition({
    issueId,
    repo,
    reworkVerdictId: "4".repeat(64),
    snapshot: snapshot([]),
    stream: "windows",
    terminalSetId: "5".repeat(64),
  });

  assert.equal(captured.command, "move_project_bw_issue_to_ready");
  assert.deepEqual(captured.args.input, {
    issueId,
    repo,
    reworkVerdictId: "4".repeat(64),
    stream: "windows",
    terminalSetId: "5".repeat(64),
  });
});
