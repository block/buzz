import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const dir = mkdtempSync(path.join(tmpdir(), "triage-classify-"));
const file = path.join(dir, "data.json");
writeFileSync(file, `${JSON.stringify({})}\n`);
process.env.TRIAGE_DATA_FILE = file;
process.env.TRIAGE_LLM = "0";

const { recordTranscript, resetStore } = await import("./store.mjs");
const { triageBatch } = await import("./classify.mjs");

const PUBKEY = "orchestration";

test.after(() => {
  resetStore();
  rmSync(dir, { recursive: true, force: true });
});

function message(overrides = {}) {
  return {
    eventId: "e1",
    channelId: "c1",
    channelName: "engineering",
    threadRootId: null,
    authorPubkey: "vlad",
    authorLabel: "Vlad",
    content: "@jacob can you review the migration before the release",
    createdAt: 100,
    isDm: false,
    isMention: true,
    isSelf: false,
    ...overrides,
  };
}

test("a batch produces fibres, ignores, and a lane for every fibre", async () => {
  const messages = [
    message({ eventId: "ask", isMention: true }),
    message({
      eventId: "chatter",
      isMention: false,
      content: "lol",
      authorPubkey: "zhenya",
      authorLabel: "Zhenya",
    }),
    message({
      eventId: "mine",
      isMention: false,
      isSelf: true,
      authorPubkey: PUBKEY,
      content: "I will pick this up after standup",
    }),
  ];
  recordTranscript(PUBKEY, messages);

  const result = await triageBatch({
    pubkey: PUBKEY,
    messages,
    fibres: [],
    now: 500,
  });

  assert.equal(result.fibres.length, 1);
  assert.deepEqual(result.fibres[0].artifacts.map((a) => a.eventId), ["ask"]);
  assert.ok(["important", "hot", "other"].includes(result.fibres[0].lane));
  assert.equal(typeof result.fibres[0].engagement, "number");

  assert.deepEqual(
    result.ignored.map((entry) => entry.eventId).sort(),
    ["chatter", "mine"],
  );
  assert.equal(
    result.ignored.find((entry) => entry.eventId === "mine").reason,
    "You wrote this",
  );
  assert.deepEqual(result.ingestedEventIds, ["ask", "chatter", "mine"]);
});

test("a follow-up joins the existing fibre and rescores it upward", async () => {
  const pubkey = "followups";
  const first = message({ eventId: "ping1", threadRootId: "t", createdAt: 100 });
  recordTranscript(pubkey, [first]);

  const opened = await triageBatch({
    pubkey,
    messages: [first],
    fibres: [],
    now: 500,
  });
  const fibreId = opened.fibres[0].id;
  const scoreBefore = opened.fibres[0].score;

  const bump = message({
    eventId: "ping2",
    threadRootId: "t",
    createdAt: 200,
    content: "bump — any update on this?",
  });
  recordTranscript(pubkey, [bump]);

  const after = await triageBatch({
    pubkey,
    messages: [bump],
    fibres: opened.fibres,
    now: 600,
  });

  assert.equal(after.fibres.length, 1);
  assert.equal(after.fibres[0].id, fibreId);
  assert.deepEqual(
    after.fibres[0].artifacts.map((artifact) => artifact.eventId),
    ["ping1", "ping2"],
  );
  assert.ok(
    after.fibres[0].score > scoreBefore,
    `expected ${after.fibres[0].score} > ${scoreBefore}`,
  );
});

test("an ignored message addressed to the viewer is flagged in the ledger", async () => {
  const pubkey = "ledger";
  const ack = message({ eventId: "ack", content: "thanks!" });
  recordTranscript(pubkey, [ack]);

  const result = await triageBatch({
    pubkey,
    messages: [ack],
    fibres: [],
    now: 500,
  });

  assert.equal(result.fibres.length, 0);
  assert.equal(result.ignored[0].wasAddressed, true);
  assert.equal(result.ignored[0].preview, "thanks!");
});
