import assert from "node:assert/strict";
import test from "node:test";

import {
  parseCatchUpRecap,
  parseSharedCatchUp,
  parseSharedCatchUpList,
} from "./tauriThreadCatchUp.ts";

const ROOT = "f".repeat(64);
const A = "a".repeat(64);
const B = "b".repeat(64);
const SIGNER = "c".repeat(64);

function recap(overrides = {}) {
  return {
    version: 1,
    channelId: "0b9a2c5e-1111-4222-8333-444455556666",
    rootEventId: ROOT,
    since: 100,
    until: 200,
    points: [
      { id: "p1", category: "decision", text: "Ship Friday", sourceIds: [A] },
    ],
    sourceIds: [A, B],
    coverage: {
      messageCount: 2,
      inputMessageCount: 2,
      truncated: false,
      warnings: [],
    },
    ...overrides,
  };
}

function shared(overrides = {}) {
  return {
    artifactId: "6f1c0a52-0000-5000-8000-000000000001",
    eventId: "d".repeat(64),
    authorPubkey: SIGNER,
    createdAt: 300,
    recap: recap(),
    corrections: [],
    historyComplete: true,
    ...overrides,
  };
}

test("recap ids are lowercased and kept", () => {
  const parsed = parseCatchUpRecap(
    recap({
      rootEventId: ROOT.toUpperCase(),
      sourceIds: [A.toUpperCase(), B],
      points: [
        {
          id: "p1",
          category: "question",
          text: "Who?",
          sourceIds: [A.toUpperCase()],
        },
      ],
    }),
  );
  assert.equal(parsed.rootEventId, ROOT);
  assert.deepEqual(parsed.points[0].sourceIds, [A]);
});

const malformed = [
  ["wrong version", recap({ version: 2 })],
  [
    "unknown category",
    recap({
      points: [{ id: "p1", category: "action", text: "x", sourceIds: [A] }],
    }),
  ],
  [
    "citation outside the input",
    recap({
      points: [
        {
          id: "p1",
          category: "decision",
          text: "x",
          sourceIds: ["e".repeat(64)],
        },
      ],
    }),
  ],
  [
    "uncited point",
    recap({
      points: [{ id: "p1", category: "decision", text: "x", sourceIds: [] }],
    }),
  ],
  [
    "blank text",
    recap({
      points: [{ id: "p1", category: "decision", text: "  ", sourceIds: [A] }],
    }),
  ],
  [
    "duplicate point id",
    recap({
      points: [
        { id: "p1", category: "decision", text: "x", sourceIds: [A] },
        { id: "p1", category: "question", text: "y", sourceIds: [B] },
      ],
    }),
  ],
  ["non-hex source", recap({ sourceIds: ["not-an-id"] })],
  ["until before since", recap({ since: 300, until: 200 })],
  [
    "too many points",
    recap({
      points: Array.from({ length: 13 }, (_, i) => ({
        id: `p${i}`,
        category: "decision",
        text: "x",
        sourceIds: [A],
      })),
    }),
  ],
];

const HASH_A = "1".repeat(64);
const HASH_B = "2".repeat(64);

test("source revisions round-trip byte for byte into the share payload", () => {
  const revisions = { [A]: HASH_A };
  const parsed = parseCatchUpRecap(recap({ sourceRevisions: revisions }));
  assert.deepEqual(parsed.sourceRevisions, revisions);
  assert.notEqual(parsed.sourceRevisions, revisions, "copied, not aliased");
  // Shared recaps carry the same map, so a re-share sends what native signed.
  const again = parseSharedCatchUp(shared({ recap: parsed }));
  assert.deepEqual(again.recap.sourceRevisions, revisions);
  assert.deepEqual(JSON.parse(JSON.stringify(parsed)), {
    ...recap(),
    sourceRevisions: revisions,
  });
});

test("legacy recaps without source revisions stay valid and gain no key", () => {
  const parsed = parseCatchUpRecap(recap());
  assert.equal(Object.hasOwn(parsed, "sourceRevisions"), false);
});

test("source revisions may omit uncited sources", () => {
  const parsed = parseCatchUpRecap(recap({ sourceRevisions: { [A]: HASH_A } }));
  assert.deepEqual(parsed.sourceRevisions, { [A]: HASH_A });
});

const tooManyIds = Array.from({ length: 401 }, (_, index) =>
  index.toString(16).padStart(64, "0"),
);
const malformedRevisions = [
  ["a non-object map", "abc"],
  ["an array map", [HASH_A]],
  ["an uppercase key", { [A.toUpperCase()]: HASH_A }],
  ["an uppercase hash", { [A]: HASH_A.replace(/1/g, "F") }],
  ["a short hash", { [A]: "1".repeat(63) }],
  ["a non-string hash", { [A]: 1 }],
  ["a key outside sourceIds", { [A]: HASH_A, ["9".repeat(64)]: HASH_B }],
  ["an uncited input source", { [A]: HASH_A, [B]: HASH_B }],
  ["a cited source with no revision", { [B]: HASH_B }],
];
for (const [name, sourceRevisions] of malformedRevisions) {
  test(`recap rejects source revisions with ${name}`, () => {
    assert.throws(
      () => parseCatchUpRecap(recap({ sourceRevisions })),
      /sourceRevisions/,
    );
  });
}

test("recap rejects more than 400 sources or source revisions", () => {
  assert.throws(
    () => parseCatchUpRecap(recap({ sourceIds: [A, ...tooManyIds] })),
    /sourceIds/,
  );
  const map = Object.fromEntries(tooManyIds.map((id) => [id, HASH_A]));
  assert.throws(
    () => parseCatchUpRecap(recap({ sourceRevisions: map })),
    /sourceRevisions/,
  );
});

for (const [name, value] of malformed) {
  test(`recap rejects ${name}`, () => {
    assert.throws(() => parseCatchUpRecap(value), /malformed/);
  });
}

test("shared recap rejects corrections to points it does not contain", () => {
  assert.throws(
    () =>
      parseSharedCatchUp(
        shared({
          corrections: [
            {
              eventId: "e".repeat(64),
              pointId: "p9",
              text: "x",
              authorPubkey: SIGNER,
              createdAt: 400,
            },
          ],
        }),
      ),
    /correction.pointId/,
  );
});

test("list keeps valid recaps, counts unreadable ones, and reports truncation", () => {
  const list = parseSharedCatchUpList({
    recaps: [shared(), { ...shared(), recap: recap({ version: 9 }) }],
    unreadableCount: 2,
    complete: false,
  });
  assert.equal(list.recaps.length, 1);
  assert.equal(list.unreadableCount, 3);
  assert.equal(list.complete, false);
});

test("list rejects the pre-amendment bare array shape", () => {
  assert.throws(() => parseSharedCatchUpList([shared()]), /artifacts/);
});
