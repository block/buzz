import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyCanvasBoardCard,
  parseCanvasBoard,
  resolveChannelViewMode,
} from "./canvasBoard.ts";

test("parseCanvasBoard turns level-two sections into durable cards", () => {
  const board = parseCanvasBoard(`# Dispatch — Open Studio 001

Bring one seed and leave with one artifact.

## This week at Sweet Works

Open Studio 001 is active.

## Start here

1. Read the welcome.
2. Open a Workshop thread.

## Finished example

[Open the magic mirror](https://example.com)
`);

  assert.equal(board.title, "Dispatch — Open Studio 001");
  assert.equal(
    board.introduction,
    "Bring one seed and leave with one artifact.",
  );
  assert.deepEqual(
    board.cards.map(({ kind, title }) => ({ kind, title })),
    [
      { kind: "now", title: "This week at Sweet Works" },
      { kind: "welcome", title: "Start here" },
      { kind: "artifact", title: "Finished example" },
    ],
  );
});

test("parseCanvasBoard leaves headings inside fences in card bodies", () => {
  const board = parseCanvasBoard(`## Notes

\`\`\`md
## Not a card
\`\`\`

## Next action

Ship the proof.
`);

  assert.equal(board.cards.length, 2);
  assert.match(board.cards[0].body, /## Not a card/u);
  assert.equal(board.cards[1].kind, "invitation");
});

test("parseCanvasBoard does not close a longer fence with a shorter example fence", () => {
  const board = parseCanvasBoard(`## Notes

\`\`\`\`md
\`\`\`md
## Not a card
\`\`\`
\`\`\`\`

## Next action

Ship the proof.
`);

  assert.equal(board.cards.length, 2);
  assert.match(board.cards[0].body, /## Not a card/u);
  assert.equal(board.cards[1].title, "Next action");
});

test("parseCanvasBoard falls back to one overview card", () => {
  const board = parseCanvasBoard(
    "# A small room\n\nEverything useful lives here.",
  );

  assert.equal(board.title, "A small room");
  assert.equal(board.introduction, "");
  assert.deepEqual(board.cards, [
    {
      body: "Everything useful lives here.",
      id: "overview-1",
      kind: "welcome",
      title: "Overview",
    },
  ]);
});

test("classifyCanvasBoardCard keeps stewardship language visible", () => {
  assert.equal(classifyCanvasBoardCard("People and stewards"), "people");
  assert.equal(classifyCanvasBoardCard("Source and story boundary"), "note");
});

test("resolveChannelViewMode makes Dispatch board-first without hiding targets", () => {
  assert.deepEqual(
    resolveChannelViewMode({
      channelName: "Dispatch",
      channelType: "stream",
      explicitView: null,
      hasCanvas: true,
      hasRouteTarget: false,
    }),
    { boardAvailable: true, mode: "board" },
  );

  assert.deepEqual(
    resolveChannelViewMode({
      channelName: "Dispatch",
      channelType: "stream",
      explicitView: "board",
      hasCanvas: true,
      hasRouteTarget: true,
    }),
    { boardAvailable: true, mode: "stream" },
  );

  assert.deepEqual(
    resolveChannelViewMode({
      channelName: "The Workshop",
      channelType: "stream",
      explicitView: null,
      hasCanvas: true,
      hasRouteTarget: false,
    }),
    { boardAvailable: true, mode: "stream" },
  );
});
