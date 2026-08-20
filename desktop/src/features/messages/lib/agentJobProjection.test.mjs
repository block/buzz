import assert from "node:assert/strict";
import test from "node:test";

import {
  KIND_JOB_ACCEPTED,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_REQUEST,
  KIND_JOB_RESULT,
} from "@/shared/constants/kinds";
import { formatTimelineMessages } from "./formatTimelineMessages.ts";
import { reduceAgentJobEvents } from "./agentJobProjection.ts";

const JOB_ID = "123e4567-e89b-42d3-a456-426614174000";
const CHANNEL_ID = "36411e44-0e2d-4cfe-bd6e-567eb169db9f";
const REQUESTER = "11".repeat(32);
const AGENT = "22".repeat(32);
const SOURCE_ID = "33".repeat(32);
const REQUEST_ID = "44".repeat(32);

function event(kind, idByte, createdAt, content, tags, pubkey = AGENT) {
  return {
    id: idByte.repeat(64),
    pubkey,
    created_at: createdAt,
    kind,
    tags,
    content: JSON.stringify(content),
    sig: "sig",
  };
}

function request() {
  return event(
    KIND_JOB_REQUEST,
    "4",
    1_700_000_000,
    {
      schema: 1,
      driver: "lh",
      argv: ["lockdown", "run", "--issue", "JAC-575"],
      cwd: "/workspace",
      summary: "Repair JAC-575",
    },
    [
      ["h", CHANNEL_ID],
      ["p", AGENT],
      ["job", JOB_ID],
      ["e", SOURCE_ID],
    ],
    REQUESTER,
  );
}

function accepted() {
  return event(
    KIND_JOB_ACCEPTED,
    "5",
    1_700_000_001,
    {
      schema: 1,
      job: JOB_ID,
      attempt: 1,
      state: "accepted",
      accepted_at: "2023-11-14T22:13:21Z",
    },
    [
      ["h", CHANNEL_ID],
      ["p", REQUESTER],
      ["job", JOB_ID],
      ["e", REQUEST_ID],
    ],
  );
}

function progress(seq = 1) {
  return event(
    KIND_JOB_PROGRESS,
    "6",
    1_700_000_002,
    {
      schema: 1,
      job: JOB_ID,
      attempt: 1,
      seq,
      state: "running",
      summary: "Running receipt verification",
      artifacts: [],
    },
    [
      ["h", CHANNEL_ID],
      ["p", REQUESTER],
      ["job", JOB_ID],
      ["e", REQUEST_ID],
      ["seq", String(seq)],
    ],
  );
}

function result() {
  return event(
    KIND_JOB_RESULT,
    "7",
    1_700_000_003,
    {
      schema: 1,
      job: JOB_ID,
      attempt: 1,
      state: "succeeded",
      exit_code: 0,
      summary: "JAC-575 repaired",
      artifacts: [
        {
          name: "receipt.json",
          uri: "https://relay.example/artifacts/jac-575-receipt",
          sha256: "ab".repeat(32),
        },
      ],
      finished_at: "2023-11-14T22:13:23Z",
    },
    [
      ["h", CHANNEL_ID],
      ["p", REQUESTER],
      ["job", JOB_ID],
      ["e", REQUEST_ID],
    ],
  );
}

function error() {
  return event(
    KIND_JOB_ERROR,
    "8",
    1_700_000_004,
    {
      schema: 1,
      job: JOB_ID,
      attempt: 1,
      state: "failed",
      code: "runner_failed",
      summary: "Runner failed",
      retryable: false,
      artifacts: [],
      finished_at: "2023-11-14T22:13:24Z",
    },
    [
      ["h", CHANNEL_ID],
      ["p", REQUESTER],
      ["job", JOB_ID],
      ["e", REQUEST_ID],
    ],
  );
}

test("ordered and out-of-order duplicate deliveries reduce to the same job view", () => {
  const ordered = [request(), accepted(), progress(), result()];
  const shuffledWithDuplicates = [
    result(),
    progress(),
    request(),
    accepted(),
    progress(),
    request(),
  ];

  const orderedView =
    reduceAgentJobEvents(ordered).viewsByRepresentativeEventId.get(REQUEST_ID);
  const shuffledView = reduceAgentJobEvents(
    shuffledWithDuplicates,
  ).viewsByRepresentativeEventId.get(REQUEST_ID);

  assert.deepEqual(shuffledView, orderedView);
  assert.equal(orderedView.state, "succeeded");
  assert.equal(orderedView.summary, "JAC-575 repaired");
  assert.equal(orderedView.artifacts.length, 1);
});

test("a valid signed lifecycle renders as one deterministic timeline job card", () => {
  const messages = formatTimelineMessages(
    [request(), accepted(), progress()],
    null,
    undefined,
    null,
  );

  assert.equal(messages.length, 1);
  assert.equal(messages[0].id, REQUEST_ID);
  assert.equal(messages[0].jobView?.state, "running");
  assert.equal(messages[0].jobView?.sourceEventId, SOURCE_ID);
  assert.equal(messages[0].jobView?.targetPubkey, AGENT);
});

test("a second terminal invalidates the chain instead of choosing a winner", () => {
  const events = [request(), accepted(), result(), error()];
  const projection = reduceAgentJobEvents(events);
  assert.equal(projection.viewsByRepresentativeEventId.size, 0);
  assert.equal(projection.collapsedEventIds.size, 0);

  const messages = formatTimelineMessages(events, null, undefined, null);
  assert.equal(messages.length, 4);
  assert.ok(messages.every((message) => message.jobView == null));
});

test("invalid linkage and incomplete requests remain raw timeline events", () => {
  const brokenProgress = progress();
  brokenProgress.tags = brokenProgress.tags.map((tag) =>
    tag[0] === "e" ? ["e", "99".repeat(32)] : tag,
  );
  const invalid = [request(), accepted(), brokenProgress];

  assert.equal(
    reduceAgentJobEvents(invalid).viewsByRepresentativeEventId.size,
    0,
  );
  const invalidMessages = formatTimelineMessages(
    invalid,
    null,
    undefined,
    null,
  );
  assert.equal(invalidMessages.length, 3);
  assert.ok(invalidMessages.every((message) => message.jobView == null));

  const incompleteMessages = formatTimelineMessages(
    [request()],
    null,
    undefined,
    null,
  );
  assert.equal(incompleteMessages.length, 1);
  assert.equal(incompleteMessages[0].jobView, undefined);
});
