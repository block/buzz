import assert from "node:assert/strict";
import test from "node:test";

import {
  appendCanvasBoardCard,
  classifyCanvasBoardCard,
  parseCanvasBoard,
  reorderCanvasBoardCard,
  resolveChannelViewMode,
  updateCanvasBoardCard,
  validateCanvasBoardCardDraft,
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

test("appendCanvasBoardCard preserves the board preamble and adds one card", () => {
  const content = `# Dispatch

Shared introduction.

## Start here

Read the welcome.
`;

  const updated = appendCanvasBoardCard(content, {
    body: "Bring one seed.",
    title: "Next action",
  });
  const board = parseCanvasBoard(updated);

  assert.equal(board.title, "Dispatch");
  assert.equal(board.introduction, "Shared introduction.");
  assert.deepEqual(
    board.cards.map(({ body, title }) => ({ body, title })),
    [
      { body: "Read the welcome.", title: "Start here" },
      { body: "Bring one seed.", title: "Next action" },
    ],
  );
});

test("updateCanvasBoardCard edits only the selected source section", () => {
  const content = `# Dispatch

## Start here

Old instructions.

## Finished example

Keep this intact.
`;

  const updated = updateCanvasBoardCard(content, "start-here-1", {
    body: "New **Markdown** instructions.",
    title: "Start here now",
  });
  assert.ok(updated);

  const board = parseCanvasBoard(updated);
  assert.deepEqual(
    board.cards.map(({ body, title }) => ({ body, title })),
    [
      {
        body: "New **Markdown** instructions.",
        title: "Start here now",
      },
      { body: "Keep this intact.", title: "Finished example" },
    ],
  );
});

test("updateCanvasBoardCard turns a fallback overview into a durable section", () => {
  const updated = updateCanvasBoardCard(
    "# A small room\n\nEverything useful lives here.\n",
    "overview-1",
    {
      body: "The overview is now editable from the Board.",
      title: "Start here",
    },
  );
  assert.ok(updated);

  const board = parseCanvasBoard(updated);
  assert.equal(board.title, "A small room");
  assert.equal(board.introduction, "");
  assert.deepEqual(
    board.cards.map(({ body, title }) => ({ body, title })),
    [
      {
        body: "The overview is now editable from the Board.",
        title: "Start here",
      },
    ],
  );
});

test("reorderCanvasBoardCard moves raw fenced sections without losing content", () => {
  const content = `# Dispatch

## Notes

\`\`\`\`md
\`\`\`md
## Not a card
\`\`\`
\`\`\`\`

## Next action

Ship the proof.

## Finished example

Keep this too.
`;

  const updated = reorderCanvasBoardCard(
    content,
    "finished-example-3",
    "notes-1",
  );
  assert.ok(updated);

  const board = parseCanvasBoard(updated);
  assert.deepEqual(
    board.cards.map(({ title }) => title),
    ["Finished example", "Notes", "Next action"],
  );
  assert.match(board.cards[1].body, /## Not a card/u);
  assert.match(board.cards[2].body, /Ship the proof/u);
});

test("canvas card draft validation rejects nested level-two card headings", () => {
  assert.equal(
    validateCanvasBoardCardDraft({ body: "Body", title: "" }),
    "Add a card title.",
  );
  assert.equal(
    validateCanvasBoardCardDraft({ body: "Body", title: "x".repeat(121) }),
    "Keep the card title to 120 characters or fewer.",
  );
  assert.equal(
    validateCanvasBoardCardDraft({
      body: "## Accidental second card",
      title: "First card",
    }),
    "Use level-three headings inside a card. Level-two headings create separate cards.",
  );
  assert.equal(
    validateCanvasBoardCardDraft({
      body: "```md\n## Safe example\n```",
      title: "Code sample",
    }),
    null,
  );
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
