import assert from "node:assert/strict";
import test from "node:test";

import {
  projectSharedCatchUpPoints,
  sharedCatchUpAttribution,
  sharedCatchUpUpdatedAt,
  upsertSharedCatchUp,
} from "./threadCatchUpArtifact.ts";

const SOURCE = "a".repeat(64);
const SHARER = "b".repeat(64);
const ALICE = "c".repeat(64);
const BOB = "d".repeat(64);

function shared(overrides = {}) {
  return {
    artifactId: "artifact-1",
    eventId: "e".repeat(64),
    authorPubkey: SHARER,
    createdAt: 100,
    recap: {
      points: [
        {
          id: "p1",
          category: "decision",
          text: "Ship Friday",
          sourceIds: [SOURCE],
        },
        {
          id: "p2",
          category: "question",
          text: "Who owns QA?",
          sourceIds: [SOURCE],
        },
      ],
    },
    corrections: [],
    historyComplete: true,
    ...overrides,
  };
}

test("points keep generated text first and the newest correction current", () => {
  const views = projectSharedCatchUpPoints(
    shared({
      corrections: [
        // Native chain order, deliberately not sorted by time.
        {
          eventId: "1".repeat(64),
          pointId: "p1",
          text: "Ship Monday",
          authorPubkey: ALICE,
          createdAt: 300,
        },
        {
          eventId: "2".repeat(64),
          pointId: "p1",
          text: "Ship Tuesday",
          authorPubkey: BOB,
          createdAt: 200,
        },
      ],
    }),
  );
  assert.equal(views[0].currentText, "Ship Tuesday");
  assert.equal(views[0].corrected, true);
  assert.deepEqual(
    views[0].history.map((entry) => [
      entry.kind,
      entry.text,
      entry.authorPubkey,
    ]),
    [
      ["generated", "Ship Friday", null],
      ["correction", "Ship Monday", ALICE],
      ["correction", "Ship Tuesday", BOB],
    ],
  );
  assert.equal(views[1].currentText, "Who owns QA?");
  assert.equal(views[1].corrected, false);
  assert.equal(views[1].history.length, 1);
});

test("incomplete history never names the earliest retrieved signer as sharer", () => {
  assert.deepEqual(
    sharedCatchUpAttribution(shared({ historyComplete: false })),
    {
      kind: "incomplete",
    },
  );
  assert.deepEqual(sharedCatchUpAttribution(shared()), {
    kind: "sharer",
    pubkey: SHARER,
    createdAt: 100,
  });
  const [view] = projectSharedCatchUpPoints(shared({ historyComplete: false }));
  assert.equal(view.history[0].createdAt, null);
});

test("upsert replaces by identity and moves the changed recap first", () => {
  const first = shared({ artifactId: "one" });
  const second = shared({ artifactId: "two" });
  const updated = shared({ artifactId: "one", eventId: "f".repeat(64) });
  const list = upsertSharedCatchUp(
    { recaps: [second, first], unreadableCount: 1, complete: false },
    updated,
  );
  assert.deepEqual(
    list.recaps.map((entry) => [entry.artifactId, entry.eventId]),
    [
      ["one", "f".repeat(64)],
      ["two", "e".repeat(64)],
    ],
  );
  assert.equal(list.unreadableCount, 1);
  assert.equal(list.complete, false);
  assert.deepEqual(upsertSharedCatchUp(undefined, first).complete, true);
});

test("updated time is the newest correction or the share", () => {
  assert.equal(sharedCatchUpUpdatedAt(shared()), 100);
  assert.equal(
    sharedCatchUpUpdatedAt(
      shared({
        corrections: [
          {
            eventId: "1".repeat(64),
            pointId: "p1",
            text: "x",
            authorPubkey: ALICE,
            createdAt: 250,
          },
        ],
      }),
    ),
    250,
  );
});
