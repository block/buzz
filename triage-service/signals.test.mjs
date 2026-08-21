import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const dir = mkdtempSync(path.join(tmpdir(), "triage-signals-"));
const file = path.join(dir, "data.json");
writeFileSync(file, `${JSON.stringify({})}\n`);
process.env.TRIAGE_DATA_FILE = file;

const { recordTranscript, resetStore } = await import("./store.mjs");
const {
  engagementScore,
  fibreSignals,
  isFollowUp,
  messageContext,
  scoreAdjustments,
} = await import("./signals.mjs");

const SELF = "self";
const THEM = "them";

test.after(() => {
  resetStore();
  rmSync(dir, { recursive: true, force: true });
});

function message(overrides = {}) {
  return {
    eventId: "e",
    channelId: "c1",
    channelName: "engineering",
    threadRootId: null,
    authorPubkey: THEM,
    authorLabel: "Them",
    content: "hello",
    createdAt: 100,
    isMention: false,
    isSelf: false,
    ...overrides,
  };
}

function fibre(overrides = {}) {
  return {
    id: "f1",
    channelId: "c1",
    channelName: "engineering",
    isDm: false,
    artifacts: [],
    ...overrides,
  };
}

test("a tagged message in a thread gets the whole thread as context", () => {
  const pubkey = "ctx-thread";
  recordTranscript(pubkey, [
    message({ eventId: "root", createdAt: 10 }),
    message({ eventId: "r1", createdAt: 20, threadRootId: "root" }),
    message({ eventId: "noise", createdAt: 25 }),
    message({
      eventId: "ping",
      createdAt: 30,
      threadRootId: "root",
      isMention: true,
    }),
  ]);

  const context = messageContext(
    pubkey,
    message({ eventId: "ping", threadRootId: "root", isMention: true }),
  );

  assert.equal(context.scope, "thread");
  assert.deepEqual(
    context.messages.map((row) => row.eventId),
    ["root", "r1"],
  );
});

test("an untagged message gets the recent channel window", () => {
  const pubkey = "ctx-channel";
  recordTranscript(pubkey, [
    message({ eventId: "a", createdAt: 10 }),
    message({ eventId: "b", createdAt: 20 }),
    message({ eventId: "c", createdAt: 30 }),
  ]);

  const context = messageContext(pubkey, message({ eventId: "c" }));
  assert.equal(context.scope, "channel");
  assert.deepEqual(
    context.messages.map((row) => row.eventId),
    ["a", "b"],
  );
});

test("unanswered pings accumulate until the viewer replies", () => {
  const pubkey = "pings";
  recordTranscript(pubkey, [
    message({ eventId: "p1", createdAt: 10, threadRootId: "t", isMention: true }),
    message({ eventId: "p2", createdAt: 20, threadRootId: "t", isMention: true }),
  ]);

  const target = fibre({
    artifacts: [{ eventId: "p1", threadRootId: "t", createdAt: 10 }],
  });
  assert.equal(fibreSignals(pubkey, target).unansweredPings, 2);

  recordTranscript(pubkey, [
    message({
      eventId: "mine",
      createdAt: 30,
      threadRootId: "t",
      isSelf: true,
      authorPubkey: SELF,
    }),
  ]);

  const after = fibreSignals(pubkey, target);
  assert.equal(after.unansweredPings, 0);
  assert.equal(after.hasReplyFromViewer, true);
});

test("a busy thread with several people scores as engaging", () => {
  const pubkey = "velocity";
  const rows = [];
  for (let index = 0; index < 8; index += 1) {
    rows.push(
      message({
        eventId: `m${index}`,
        createdAt: 1_000 + index * 60,
        threadRootId: "t",
        authorPubkey: `person-${index % 3}`,
      }),
    );
  }
  recordTranscript(pubkey, rows);

  const signals = fibreSignals(
    pubkey,
    fibre({ artifacts: [{ eventId: "m0", threadRootId: "t", createdAt: 1_000 }] }),
  );

  assert.equal(signals.velocity.messages, 8);
  assert.equal(signals.velocity.participants, 3);
  assert.equal(signals.engagement, 72);
});

test("engagementScore rewards volume and distinct participants", () => {
  assert.equal(engagementScore({ messages: 0, participants: 0 }), 0);
  assert.equal(engagementScore({ messages: 3, participants: 1 }), 18);
  assert.equal(engagementScore({ messages: 100, participants: 100 }), 100);
});

test("past involvement is detected from earlier self-authored messages", () => {
  const pubkey = "involvement";
  recordTranscript(pubkey, [
    message({
      eventId: "old",
      createdAt: 10,
      threadRootId: "t",
      isSelf: true,
      authorPubkey: SELF,
    }),
    message({ eventId: "new", createdAt: 50, threadRootId: "t" }),
  ]);

  const signals = fibreSignals(
    pubkey,
    fibre({ artifacts: [{ eventId: "new", threadRootId: "t", createdAt: 50 }] }),
  );

  assert.equal(signals.pastInvolvement.thread, true);
  assert.equal(signals.pastInvolvement.any, true);
});

test("isFollowUp matches bumps but not substantive messages", () => {
  assert.equal(isFollowUp("bump"), true);
  assert.equal(isFollowUp("  any update on this?"), true);
  assert.equal(isFollowUp("still blocked on the migration"), true);
  assert.equal(isFollowUp("Can you review the migration?"), false);
});

test("scoreAdjustments guarantees the movements we do not trust to the model", () => {
  const raised = scoreAdjustments({
    unansweredPings: 3,
    followUpOnly: false,
    pastInvolvement: { any: true },
    incidentOngoing: true,
  });
  assert.equal(raised.delta, 25 + 10 + 25);

  const lowered = scoreAdjustments({
    unansweredPings: 1,
    followUpOnly: true,
    pastInvolvement: { any: false },
    incidentOngoing: false,
  });
  assert.equal(lowered.delta, -15);
});

test("an incident channel only counts as ongoing while it is moving", () => {
  const pubkey = "incident";
  recordTranscript(pubkey, [
    message({ eventId: "i1", channelId: "c9", createdAt: 1_000 }),
  ]);

  const target = fibre({
    channelId: "c9",
    channelName: "incident-payments",
    artifacts: [{ eventId: "i1", channelId: "c9", createdAt: 1_000 }],
  });

  assert.equal(fibreSignals(pubkey, target, 1_500).incidentOngoing, true);
  assert.equal(fibreSignals(pubkey, target, 100_000).incidentOngoing, false);
  assert.equal(fibreSignals(pubkey, target, 100_000).incidentChannel, true);
});
