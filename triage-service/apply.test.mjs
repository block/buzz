import assert from "node:assert/strict";
import test from "node:test";

import {
  applyFibreActions,
  deriveLane,
  messageToArtifact,
} from "./apply.mjs";

const MSG_A = {
  eventId: "a",
  channelId: "c1",
  channelName: "war-room",
  threadRootId: "root",
  authorPubkey: "vlad",
  authorLabel: "Vlad",
  content: "please run the scripts",
  createdAt: 1,
  isDm: false,
};

const MSG_B = {
  eventId: "b",
  channelId: "c1",
  channelName: "war-room",
  threadRootId: "root",
  authorPubkey: "jacob",
  authorLabel: "jacob",
  content: "on it before standup",
  createdAt: 2,
  isDm: false,
};

test("create adds a fibre from the referenced messages", () => {
  const { fibres, changes } = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [
      {
        type: "create",
        kind: "ask",
        title: "Run the scripts",
        summary: "Vlad needs the scripts run",
        why: "Direct mention",
        score: 84,
        eventIds: ["a"],
      },
    ],
    now: 10,
  });

  assert.equal(fibres.length, 1);
  assert.equal(fibres[0].kind, "ask");
  assert.equal(fibres[0].status, "open");
  assert.equal(fibres[0].score, 84);
  assert.equal(fibres[0].artifacts.length, 1);
  assert.equal(fibres[0].people[0].label, "Vlad");
  assert.deepEqual(
    changes.map((change) => change.type),
    ["create"],
  );
});

test("lanes are tried in order: important, then hot, then other", () => {
  assert.equal(deriveLane(90, 0), "important");
  assert.equal(deriveLane(90, 95), "important");
  assert.equal(deriveLane(71, 0), "important");
  assert.equal(deriveLane(70, 0), "other");
  assert.equal(deriveLane(70, 60), "hot");
  assert.equal(deriveLane(10, 59), "other");
});

test("a created fibre carries an engagement and a lane", () => {
  const { fibres } = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [
      { type: "create", kind: "ask", score: 84, engagement: 20, eventIds: ["a"] },
    ],
    now: 10,
  });

  assert.equal(fibres[0].engagement, 20);
  assert.equal(fibres[0].lane, "important");
});

test("updating a fibre re-derives its lane from the new score", () => {
  const created = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [{ type: "create", kind: "ask", score: 40, eventIds: ["a"] }],
    now: 10,
  });
  assert.equal(created.fibres[0].lane, "other");

  const updated = applyFibreActions({
    fibres: created.fibres,
    messages: [MSG_B],
    actions: [
      {
        type: "update",
        fibreId: created.fibres[0].id,
        score: 88,
        eventIds: ["b"],
      },
    ],
    now: 20,
  });

  assert.equal(updated.fibres[0].score, 88);
  assert.equal(updated.fibres[0].lane, "important");
});

test("create with unknown event ids is ignored", () => {
  const { fibres } = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [{ type: "create", kind: "ask", eventIds: ["missing"] }],
    now: 10,
  });
  assert.equal(fibres.length, 0);
});

test("update attaches a new artifact and is a no-op on a duplicate event", () => {
  const created = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [
      {
        type: "create",
        kind: "ask",
        title: "Scripts",
        eventIds: ["a"],
      },
    ],
    now: 10,
  });
  const fibreId = created.fibres[0].id;

  const updated = applyFibreActions({
    fibres: created.fibres,
    messages: [MSG_B],
    actions: [{ type: "update", fibreId, eventIds: ["b"], title: "Scripts + confirm" }],
    now: 20,
  });
  assert.equal(updated.fibres[0].artifacts.length, 2);
  assert.equal(updated.fibres[0].title, "Scripts + confirm");
  assert.equal(updated.fibres[0].people.length, 2);

  const again = applyFibreActions({
    fibres: updated.fibres,
    messages: [MSG_B],
    actions: [{ type: "update", fibreId, eventIds: ["b"] }],
    now: 30,
  });
  assert.equal(again.fibres[0].artifacts.length, 2);
});

test("merge moves artifacts onto the target and dismisses the source", () => {
  const first = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [{ type: "create", kind: "ask", title: "A", eventIds: ["a"] }],
    now: 10,
  });
  const second = applyFibreActions({
    fibres: first.fibres,
    messages: [MSG_B],
    actions: [{ type: "create", kind: "commitment", title: "B", eventIds: ["b"] }],
    now: 11,
  });
  const [one, two] = second.fibres;

  const merged = applyFibreActions({
    fibres: second.fibres,
    messages: [],
    actions: [
      {
        type: "merge",
        fibreIds: [one.id, two.id],
        into: one.id,
        title: "Combined",
      },
    ],
    now: 12,
  });

  const target = merged.fibres.find((fibre) => fibre.id === one.id);
  const source = merged.fibres.find((fibre) => fibre.id === two.id);
  assert.equal(target.title, "Combined");
  assert.equal(target.artifacts.length, 2);
  assert.equal(source.status, "dismissed");
});

test("skip still records the event as ingested", () => {
  const { ingestedEventIds, fibres } = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [{ type: "skip", eventId: "a" }],
    now: 10,
  });
  assert.deepEqual(ingestedEventIds, ["a"]);
  assert.equal(fibres.length, 0);
});

test("messageToArtifact snapshots the body", () => {
  const artifact = messageToArtifact(MSG_A);
  assert.equal(artifact.eventId, "a");
  assert.equal(artifact.channelName, "war-room");
  assert.equal(artifact.content, "please run the scripts");
});

const MSG_OTHER_CHANNEL = {
  ...MSG_B,
  eventId: "c",
  channelId: "c2",
  channelName: "general",
};

test("update refuses artifacts from a different channel", () => {
  const created = applyFibreActions({
    fibres: [],
    messages: [MSG_A],
    actions: [{ type: "create", kind: "ask", title: "Scripts", eventIds: ["a"] }],
    now: 10,
  });
  const fibreId = created.fibres[0].id;
  const updated = applyFibreActions({
    fibres: created.fibres,
    messages: [MSG_OTHER_CHANNEL],
    actions: [{ type: "update", fibreId, eventIds: ["c"] }],
    now: 20,
  });
  assert.equal(updated.fibres[0].artifacts.length, 1);
  assert.equal(updated.fibres[0].artifacts[0].eventId, "a");
  assert.equal(updated.fibres[0].channelId, "c1");
});

test("create keeps only the first channel when event ids mix channels", () => {
  const { fibres } = applyFibreActions({
    fibres: [],
    messages: [MSG_A, MSG_OTHER_CHANNEL],
    actions: [{ type: "create", kind: "ask", eventIds: ["a", "c"] }],
    now: 10,
  });
  assert.equal(fibres[0].artifacts.length, 1);
  assert.equal(fibres[0].channelId, "c1");
});
