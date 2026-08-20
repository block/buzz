import assert from "node:assert/strict";
import test from "node:test";

import {
  fibreArtifactCountLabel,
  fibrePeopleLabel,
  fibreSourceLabel,
  formatFibreAge,
  latestArtifact,
  primaryThreadTarget,
} from "./fibreFormat.ts";

test("formatFibreAge uses compact units", () => {
  const now = 1_700_000_000_000;
  assert.equal(formatFibreAge(1_700_000_000 - 41 * 60, now), "41m");
  assert.equal(formatFibreAge(1_700_000_000 - 3600, now), "1h");
  assert.equal(formatFibreAge(1_700_000_000 - 3 * 86400, now), "3d");
});

test("fibreSourceLabel prefixes channels and DMs", () => {
  assert.equal(
    fibreSourceLabel({ channelName: "war-room", isDm: false }),
    "#war-room",
  );
  assert.equal(
    fibreSourceLabel({ channelName: "Fizz", isDm: true }),
    "DM · Fizz",
  );
});

test("fibreArtifactCountLabel is singular for one", () => {
  assert.equal(fibreArtifactCountLabel(1), "1 message");
  assert.equal(fibreArtifactCountLabel(3), "3 messages");
});

test("fibrePeopleLabel joins labels", () => {
  assert.equal(
    fibrePeopleLabel([
      { pubkey: "a", label: "Vlad" },
      { pubkey: "b", label: "jacob" },
    ]),
    "Vlad, jacob",
  );
});

test("primaryThreadTarget uses the latest artifact", () => {
  const target = primaryThreadTarget({
    id: "f1",
    kind: "ask",
    status: "open",
    score: 80,
    title: "x",
    summary: "",
    why: "",
    whyShort: "",
    signals: [],
    channelId: "c1",
    channelName: "general",
    isDm: false,
    people: [],
    createdAt: 1,
    updatedAt: 2,
    artifacts: [
      {
        eventId: "old",
        channelId: "c1",
        channelName: "general",
        threadRootId: "root",
        authorPubkey: "a",
        authorLabel: "A",
        content: "old",
        createdAt: 10,
      },
      {
        eventId: "new",
        channelId: "c1",
        channelName: "general",
        threadRootId: "root",
        authorPubkey: "b",
        authorLabel: "B",
        content: "new",
        createdAt: 20,
      },
    ],
  });
  assert.equal(target?.messageId, "new");
  assert.equal(
    latestArtifact(
      target
        ? [
            {
              eventId: "new",
              createdAt: 20,
              channelId: "c1",
              channelName: "g",
              threadRootId: null,
              authorPubkey: null,
              authorLabel: null,
              content: "",
            },
          ]
        : [],
    )?.eventId,
    "new",
  );
});
