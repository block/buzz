import assert from "node:assert/strict";
import test from "node:test";

import {
  applyRootTitleWrite,
  batchTitleSyncChannels,
  createThreadTitleBatcher,
  interpretTitleDraft,
  MAX_LIVE_SYNC_CHANNELS_PER_FILTER,
  THREAD_TITLES_QUERY_KEY,
  threadTitleRootQueryKey,
} from "./threadTitles.ts";

const CHANNEL = "634a9a88-e929-427c-9dc7-3b7f7194e882";
const ROOT = "a".repeat(64);
const OTHER_ROOT = "b".repeat(64);

function entry(overrides = {}) {
  return {
    author: "p".repeat(64),
    channelId: CHANNEL,
    revision: "r".repeat(64),
    rootId: ROOT,
    title: "Release checklist",
    updatedAt: 100,
    ...overrides,
  };
}

/** A batcher whose flush runs only when the test calls `flush()`. */
function manualBatcher(fetchTitles) {
  const scheduled = [];
  const load = createThreadTitleBatcher(fetchTitles, (run) =>
    scheduled.push(run),
  );
  return {
    load,
    scheduled,
    flush: () => {
      for (const run of scheduled.splice(0)) run();
    },
  };
}

test("root lookups in one tick share a single fetch", async () => {
  const calls = [];
  const { flush, load, scheduled } = manualBatcher(async (roots) => {
    calls.push(roots);
    return [entry()];
  });
  const titled = load(ROOT.toUpperCase());
  const untitled = load(OTHER_ROOT);
  const again = load(ROOT);
  assert.equal(scheduled.length, 1);
  flush();
  assert.equal((await titled)?.title, "Release checklist");
  assert.equal(await untitled, null);
  assert.equal((await again)?.title, "Release checklist");
  // Duplicate and differently-cased roots are fetched once.
  assert.deepEqual(calls, [[ROOT, OTHER_ROOT]]);

  // The next tick starts a new batch.
  const later = load(OTHER_ROOT);
  flush();
  assert.equal(await later, null);
  assert.deepEqual(calls[1], [OTHER_ROOT]);
});

test("root lookups skip ids that can't be event ids", async () => {
  const { load, scheduled } = manualBatcher(async () => {
    throw new Error("should not fetch");
  });
  assert.equal(await load("dm:alice"), null);
  assert.equal(await load("a".repeat(63)), null);
  assert.equal(scheduled.length, 0);
});

test("a failed batch rejects every lookup in it", async () => {
  const { flush, load } = manualBatcher(async () => {
    throw new Error("relay down");
  });
  const first = load(ROOT);
  const second = load(OTHER_ROOT);
  flush();
  await assert.rejects(first, /relay down/);
  await assert.rejects(second, /relay down/);
});

test("root query keys sit under the titles key and ignore case", () => {
  assert.deepEqual(threadTitleRootQueryKey(ROOT.toUpperCase()).slice(0, 1), [
    ...THREAD_TITLES_QUERY_KEY,
  ]);
  assert.deepEqual(
    threadTitleRootQueryKey(ROOT.toUpperCase()),
    threadTitleRootQueryKey(ROOT),
  );
});

test("applyRootTitleWrite sets, keeps the known revision, and clears", () => {
  const previous = entry();
  const renamed = applyRootTitleWrite(
    previous,
    { channelId: CHANNEL, rootId: ROOT.toUpperCase(), title: "Renamed" },
    200,
  );
  assert.equal(renamed?.title, "Renamed");
  assert.equal(renamed?.rootId, ROOT);
  assert.equal(renamed?.revision, previous.revision);
  assert.equal(renamed?.updatedAt, 200);
  assert.equal(
    applyRootTitleWrite(
      previous,
      { channelId: CHANNEL, rootId: ROOT, title: "Renamed", revision: "new" },
      200,
    )?.revision,
    "new",
  );
  assert.equal(
    applyRootTitleWrite(
      previous,
      { channelId: CHANNEL, rootId: ROOT, title: null },
      200,
    ),
    null,
  );
  // First title on an untitled thread.
  assert.equal(
    applyRootTitleWrite(
      null,
      { channelId: CHANNEL, rootId: ROOT, title: "New" },
      200,
    )?.revision,
    "",
  );
});

test("interpretTitleDraft trims, clears on blank, and skips no-op writes", () => {
  assert.deepEqual(interpretTitleDraft("  New  ", "Old"), {
    kind: "set",
    title: "New",
  });
  assert.deepEqual(interpretTitleDraft(" Old ", "Old"), { kind: "unchanged" });
  assert.deepEqual(interpretTitleDraft("   ", "Old"), { kind: "clear" });
  assert.deepEqual(interpretTitleDraft("", null), { kind: "unchanged" });
});

test("interpretTitleDraft bounds titles by UTF-8 bytes, not characters", () => {
  assert.equal(interpretTitleDraft("é".repeat(256), null).kind, "set");
  assert.equal(interpretTitleDraft("é".repeat(257), null).kind, "too-long");
});

test("batchTitleSyncChannels dedupes, sorts, and splits at the relay #h cap", () => {
  assert.deepEqual(batchTitleSyncChannels([]), []);
  assert.deepEqual(batchTitleSyncChannels(["b", "a", "b", ""]), [["a", "b"]]);
  const ids = Array.from(
    { length: 130 },
    (_, i) => `c${String(i).padStart(3, "0")}`,
  );
  const batches = batchTitleSyncChannels(ids.reverse());
  assert.equal(MAX_LIVE_SYNC_CHANNELS_PER_FILTER, 128);
  assert.deepEqual(
    batches.map((batch) => batch.length),
    [128, 2],
  );
  assert.equal(batches[0][0], "c000");
  assert.equal(batches[1][1], "c129");
});
