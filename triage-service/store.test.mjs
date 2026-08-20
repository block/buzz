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

const { fibresPayload, patchFibre, putFibres, resetStore } = await import(
  "./store.mjs"
);

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
