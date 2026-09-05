import assert from "node:assert/strict";
import test from "node:test";

import {
  appendCanvasBoardCard,
  buildCanvasBoardCardConversationOpener,
  canvasBoardCardConversationMarker,
  classifyCanvasBoardCard,
  classifyCanvasBoardCardType,
  parseCanvasBoard,
  reorderCanvasBoardCard,
  resolveChannelViewMode,
  updateCanvasBoardCard,
  updateCanvasBoardCardMetadata,
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
  assert.deepEqual(
    board.cards.map(({ body, id, kind, status, threadId, title, type }) => ({
      body,
      id,
      kind,
      status,
      threadId,
      title,
      type,
    })),
    [
      {
        body: "Everything useful lives here.",
        id: "overview-1",
        kind: "welcome",
        status: "backlog",
        threadId: null,
        title: "Overview",
        type: "note",
      },
    ],
  );
});

test("classifyCanvasBoardCard keeps stewardship language visible", () => {
  assert.equal(classifyCanvasBoardCard("People and stewards"), "people");
  assert.equal(classifyCanvasBoardCard("Source and story boundary"), "note");
});

test("classifyCanvasBoardCardType recognizes native workflow cards", () => {
  assert.equal(
    classifyCanvasBoardCardType("Decision: use one source"),
    "decision",
  );
  assert.equal(classifyCanvasBoardCardType("Ora Mirror project"), "project");
  assert.equal(classifyCanvasBoardCardType("Agent: Fizz"), "agent");
  assert.equal(classifyCanvasBoardCardType("A plain thought"), "note");
});

test("appendCanvasBoardCard preserves the board preamble and adds one card", () => {
  const content = `# Dispatch

Shared introduction.

## Start here

Read the welcome.
`;

  const updated = appendCanvasBoardCard(content, {
    author: "a".repeat(64),
    body: "Bring one seed.",
    id: "fresh-card-id",
    status: "doing",
    title: "Next action",
    type: "task",
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
  assert.match(updated, /<!-- buzz-board-card /u);
  assert.equal(board.cards[1].id, "fresh-card-id");
  assert.equal(board.cards[1].type, "task");
  assert.equal(board.cards[1].status, "doing");
  assert.equal(board.cards[1].author, "a".repeat(64));
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

test("card metadata gives cards durable identity, workflow state, and a thread", () => {
  const threadId = "b".repeat(64);
  const content = `# Dispatch

## Ship the proof
<!-- buzz-board-card {"id":"card-123","type":"task","status":"doing","thread":"${threadId}","author":"author-pubkey"} -->

Keep the visible body clean.
`;

  const [card] = parseCanvasBoard(content).cards;
  assert.equal(card.id, "card-123");
  assert.equal(card.type, "task");
  assert.equal(card.status, "doing");
  assert.equal(card.threadId, threadId);
  assert.equal(card.author, "author-pubkey");
  assert.equal(card.body, "Keep the visible body clean.");
  assert.equal(card.hasExplicitMetadata, true);
});

test("metadata updates preserve the human Markdown and materialize legacy cards", () => {
  const threadId = "c".repeat(64);
  const content = `# Dispatch

## Next action

Ship the proof.
`;
  const updated = updateCanvasBoardCardMetadata(content, "next-action-1", {
    status: "done",
    threadId,
    type: "decision",
  });
  assert.ok(updated);

  const [card] = parseCanvasBoard(updated).cards;
  assert.equal(card.id, "next-action-1");
  assert.equal(card.type, "decision");
  assert.equal(card.status, "done");
  assert.equal(card.threadId, threadId);
  assert.equal(card.body, "Ship the proof.");
  assert.match(updated, /<!-- buzz-board-card /u);
});

test("card conversations use a deterministic marker and readable opener", () => {
  const [card] = parseCanvasBoard(
    "## Decide next step\n\nChoose the small loop.\n",
  ).cards;
  assert.equal(
    canvasBoardCardConversationMarker(card.id),
    "magic-board-card:decide-next-step-1",
  );
  assert.equal(
    buildCanvasBoardCardConversationOpener(card, "Dispatch"),
    "## Decide next step\n\nChoose the small loop.\n\n_Conversation attached to the Dispatch board._",
  );
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
