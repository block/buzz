import assert from "node:assert/strict";
import test from "node:test";

import {
  aggregateLastActivity,
  aggregateUnreadMains,
  appendSubChannelToParentCanvas,
  applySubChannelRenames,
  indexSubChannels,
  parseSubChannelName,
  planSubChannelRenames,
  subChannelAnnouncement,
  subChannelCanvasDoc,
  subChannelName,
} from "./subChannels.ts";

function channel(id, name, lastMessageAt = null) {
  return { id, name, lastMessageAt };
}

test("parseSubChannelName_splitsOnFirstDoubleHyphen", () => {
  assert.deepEqual(parseSubChannelName("deploy-fixes--flaky-ci"), {
    parentName: "deploy-fixes",
    subSlug: "flaky-ci",
  });
});

test("parseSubChannelName_rejectsPlainNames", () => {
  assert.equal(parseSubChannelName("deploy-fixes"), null);
  assert.equal(parseSubChannelName("deploy-fixes-ci"), null);
});

test("parseSubChannelName_rejectsEmptyComponents", () => {
  assert.equal(parseSubChannelName("--sub"), null);
  assert.equal(parseSubChannelName("parent--"), null);
});

test("parseSubChannelName_nestedNameParsesAsChildOfFirstParent", () => {
  // `a--b--c` splits at the first separator; `a--b` is not a valid parent
  // name for indexing purposes, so it stays an orphan unless `a` exists.
  assert.deepEqual(parseSubChannelName("a--b--c"), {
    parentName: "a",
    subSlug: "b--c",
  });
});

test("subChannelName_roundTripsWithParse", () => {
  const name = subChannelName("deploy-fixes", "rollback-plan");
  assert.equal(name, "deploy-fixes--rollback-plan");
  assert.deepEqual(parseSubChannelName(name), {
    parentName: "deploy-fixes",
    subSlug: "rollback-plan",
  });
});

test("indexSubChannels_pairsSubsWithParents", () => {
  const parent = channel("p1", "deploy-fixes");
  const subA = channel("c1", "deploy-fixes--flaky-ci");
  const subB = channel("c2", "deploy-fixes--rollback");
  const other = channel("p2", "general");
  const index = indexSubChannels([subB, parent, other, subA]);

  assert.deepEqual(
    index.mains.map((c) => c.id),
    ["p1", "p2"],
  );
  assert.deepEqual(
    index.subsByParentId.get("p1").map((c) => c.name),
    ["deploy-fixes--flaky-ci", "deploy-fixes--rollback"],
  );
  assert.equal(index.parentIdByChildId.get("c1"), "p1");
  assert.equal(index.parentIdByChildId.get("c2"), "p1");
});

test("indexSubChannels_ordersTabsByLatestMessageThenName", () => {
  const index = indexSubChannels([
    channel("p1", "work"),
    channel("quiet-b", "work--quiet-b"),
    channel("older", "work--older", "2026-07-20T00:00:00Z"),
    channel("newest", "work--newest", "2026-07-31T00:00:00.000Z"),
    channel("quiet-a", "work--quiet-a"),
  ]);

  assert.deepEqual(
    index.subsByParentId.get("p1").map((c) => c.id),
    ["newest", "older", "quiet-a", "quiet-b"],
  );
});

test("indexSubChannels_orphanSubStaysVisibleAsMain", () => {
  const index = indexSubChannels([channel("c1", "ghost--task")]);
  assert.deepEqual(
    index.mains.map((c) => c.id),
    ["c1"],
  );
  assert.equal(index.subsByParentId.size, 0);
});

test("indexSubChannels_handlesHundredsOfSubsInOnePass", () => {
  const channels = [channel("p", "work")];
  for (let i = 0; i < 500; i += 1) {
    channels.push(channel(`c${i}`, `work--task-${String(i).padStart(3, "0")}`));
  }
  const index = indexSubChannels(channels);
  assert.equal(index.mains.length, 1);
  assert.equal(index.subsByParentId.get("p").length, 500);
  assert.equal(index.subsByParentId.get("p")[0].name, "work--task-000");
});

test("aggregateUnreadMains_bubblesSubUnreadToParent", () => {
  const index = indexSubChannels([
    channel("p1", "work"),
    channel("c1", "work--api"),
    channel("p2", "general"),
  ]);
  const unread = aggregateUnreadMains(index, new Set(["c1"]));
  assert.deepEqual([...unread], ["p1"]);
});

test("aggregateLastActivity_usesLatestSubTimestamp", () => {
  const index = indexSubChannels([
    channel("p1", "work", "2026-07-01T00:00:00Z"),
    channel("c1", "work--api", "2026-07-20T00:00:00Z"),
    channel("c2", "work--ui", "2026-07-10T00:00:00Z"),
  ]);
  const overrides = aggregateLastActivity(index);
  assert.equal(overrides.get("p1"), "2026-07-20T00:00:00Z");
});

test("subChannelAnnouncement_matchesCliFormat", () => {
  assert.equal(subChannelAnnouncement("work--api"), "→ spawned #work--api");
});

test("subChannelCanvasDoc_recordsParentAndReportBackContract", () => {
  const doc = subChannelCanvasDoc({
    parentName: "work",
    parentId: "uuid-1",
    announcementEventId: "event-1",
    task: "Build the API",
  });
  assert.match(doc, /# Sub-channel of #work/);
  assert.match(doc, /- parent: #work \(uuid-1\)/);
  assert.match(doc, /- spawned-from: event-1/);
  assert.match(doc, /- task: Build the API/);
  assert.match(doc, /thread reply to the spawn announcement/);
});

test("appendSubChannelToParentCanvas_createsSectionWhenMissing", () => {
  assert.equal(
    appendSubChannelToParentCanvas(null, "work--api", "Build API"),
    "## Sub-channels\n- #work--api — Build API\n",
  );
  assert.equal(
    appendSubChannelToParentCanvas("# Work", "work--api", "Build API"),
    "# Work\n\n## Sub-channels\n- #work--api — Build API\n",
  );
});

test("appendSubChannelToParentCanvas_appendsAfterExistingBullets", () => {
  const canvas =
    "# Work\n\n## Sub-channels\n- #work--one — One\n\nNotes\n\n## Links\n- link";
  assert.equal(
    appendSubChannelToParentCanvas(canvas, "work--two", "Two"),
    "# Work\n\n## Sub-channels\n- #work--one — One\n- #work--two — Two\n\nNotes\n\n## Links\n- link",
  );
});

test("planSubChannelRenames_rewritesEveryChildPrefix", () => {
  const channels = [
    channel("p", "work"),
    channel("c1", "work--api"),
    channel("c2", "work--ui"),
    channel("x", "workshop--misc"),
  ];
  assert.deepEqual(planSubChannelRenames(channels, "work", "platform"), [
    { channelId: "c1", newName: "platform--api" },
    { channelId: "c2", newName: "platform--ui" },
  ]);
});

test("planSubChannelRenames_noOpWhenNameUnchanged", () => {
  assert.deepEqual(
    planSubChannelRenames([channel("c1", "work--api")], "work", "work"),
    [],
  );
});

test("applySubChannelRenames_boundsConcurrencyAndCollectsFailures", async () => {
  const renames = Array.from({ length: 40 }, (_, i) => ({
    channelId: `c${i}`,
    newName: `next--task-${i}`,
  }));
  let inFlight = 0;
  let peak = 0;
  const { failed } = await applySubChannelRenames(
    renames,
    async (channelId) => {
      inFlight += 1;
      peak = Math.max(peak, inFlight);
      await new Promise((resolve) => setTimeout(resolve, 1));
      inFlight -= 1;
      if (channelId === "c7") throw new Error("boom");
    },
    4,
  );
  assert.ok(peak <= 4, `expected concurrency <= 4, saw ${peak}`);
  assert.deepEqual(
    failed.map((f) => f.channelId),
    ["c7"],
  );
});
