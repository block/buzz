import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";

const dir = mkdtempSync(path.join(tmpdir(), "triage-store-"));
const file = path.join(dir, "data.json");
writeFileSync(
  file,
  `${JSON.stringify({ fibres: {}, ingested: {}, feedback: {} })}\n`,
);
process.env.TRIAGE_DATA_FILE = file;

const {
  channelWindow,
  fibresPayload,
  patchFibre,
  putFibres,
  recordTranscript,
  resetStore,
  threadMessages,
} = await import("./store.mjs");

const PUBKEY = "pk";

function fibre(overrides = {}) {
  return {
    id: "f1",
    kind: "ask",
    status: "open",
    score: 70,
    title: "Run the scripts",
    summary: "",
    why: "",
    whyShort: "",
    signals: [],
    channelId: "c1",
    channelName: "general",
    isDm: false,
    people: [],
    artifacts: [],
    createdAt: 10,
    updatedAt: 10,
    ...overrides,
  };
}

test.after(() => {
  resetStore();
  rmSync(dir, { recursive: true, force: true });
});

test("fibresPayload includes done fibres and omits dismissed", () => {
  putFibres(PUBKEY, [
    fibre(),
    fibre({ id: "f2", status: "done", updatedAt: 30, title: "Finished ask" }),
    fibre({ id: "f3", status: "dismissed", title: "Not a fibre" }),
  ]);

  const payload = fibresPayload(PUBKEY);
  assert.equal(payload.openCount, 1);
  assert.equal(payload.doneCount, 1);
  assert.equal(payload.clearedCount, 2);
  assert.equal(payload.fibres[0].id, "f1");
  assert.equal(payload.done[0].id, "f2");
  assert.equal(
    payload.done.some((item) => item.id === "f3"),
    false,
  );
});

test("patching a fibre to done moves it into the done list", () => {
  putFibres(PUBKEY, [fibre(), fibre({ id: "f2", score: 40 })]);
  patchFibre(PUBKEY, "f1", { status: "done" });
  const payload = fibresPayload(PUBKEY);
  assert.equal(payload.openCount, 1);
  assert.equal(payload.doneCount, 1);
  assert.equal(payload.done[0].id, "f1");
  assert.equal(payload.done[0].status, "done");
});

function transcriptMessage(overrides = {}) {
  return {
    eventId: "e1",
    channelId: "c1",
    threadRootId: null,
    authorPubkey: "them",
    authorLabel: "Them",
    content: "hello",
    createdAt: 100,
    isMention: false,
    isSelf: false,
    ...overrides,
  };
}

test("channelWindow returns the messages preceding an event, oldest first", () => {
  const pubkey = "transcript-window";
  recordTranscript(pubkey, [
    transcriptMessage({ eventId: "a", createdAt: 10 }),
    transcriptMessage({ eventId: "b", createdAt: 20 }),
    transcriptMessage({ eventId: "c", createdAt: 30 }),
    transcriptMessage({ eventId: "d", createdAt: 40 }),
  ]);

  assert.deepEqual(
    channelWindow(pubkey, "c1", "d", 2).map((row) => row.eventId),
    ["b", "c"],
  );
  assert.deepEqual(channelWindow(pubkey, "c1", "a", 2), []);
});

test("recordTranscript keeps self-authored and ignored messages", () => {
  const pubkey = "transcript-self";
  recordTranscript(pubkey, [
    transcriptMessage({ eventId: "ping", createdAt: 10, isMention: true }),
    transcriptMessage({
      eventId: "reply",
      createdAt: 20,
      isSelf: true,
      authorPubkey: pubkey,
    }),
  ]);

  const rows = channelWindow(pubkey, "c1", "reply", 5);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].isMention, true);
  assert.equal(
    channelWindow(pubkey, "c1", "missing", 5).at(-1)?.isSelf,
    true,
  );
});

test("recordTranscript ignores duplicates and messages without a channel", () => {
  const pubkey = "transcript-dupes";
  recordTranscript(pubkey, [transcriptMessage({ eventId: "a", createdAt: 10 })]);
  recordTranscript(pubkey, [
    transcriptMessage({ eventId: "a", createdAt: 10 }),
    transcriptMessage({ eventId: "orphan", channelId: null }),
    transcriptMessage({ eventId: "b", createdAt: 20 }),
  ]);

  assert.deepEqual(
    channelWindow(pubkey, "c1", "missing", 10).map((row) => row.eventId),
    ["a", "b"],
  );
});

test("threadMessages collects the root and its replies", () => {
  const pubkey = "transcript-thread";
  recordTranscript(pubkey, [
    transcriptMessage({ eventId: "root", createdAt: 10 }),
    transcriptMessage({ eventId: "r1", createdAt: 20, threadRootId: "root" }),
    transcriptMessage({ eventId: "elsewhere", createdAt: 30 }),
    transcriptMessage({ eventId: "r2", createdAt: 40, threadRootId: "root" }),
  ]);

  assert.deepEqual(
    threadMessages(pubkey, "c1", "root").map((row) => row.eventId),
    ["root", "r1", "r2"],
  );
  assert.deepEqual(threadMessages(pubkey, "c1", null), []);
});
