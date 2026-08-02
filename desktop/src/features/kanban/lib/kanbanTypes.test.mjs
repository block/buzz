import assert from "node:assert/strict";
import test from "node:test";

import {
  collapseBoards,
  collapseCards,
  parseBoard,
  parseCard,
  parseColumnTag,
} from "./kanbanTypes.ts";

function ev(overrides = {}, tags = []) {
  return {
    id: "event-id",
    pubkey: "abc123",
    created_at: 1000,
    kind: 31001,
    content: "",
    sig: "sig",
    tags,
    ...overrides,
  };
}

test("parseColumnTag: parses by name, tolerates a missing wip pair", () => {
  const withWip = parseColumnTag([
    "column",
    "todo",
    "name",
    "To Do",
    "wip",
    "3",
    "order",
    "0",
  ]);
  assert.deepEqual(withWip, { id: "todo", name: "To Do", wip: 3, order: 0 });

  // No `wip` pair — the same bug class as P1's no-WIP CLI parse. Must still
  // read `name` + `order` and default wip to null.
  const noWip = parseColumnTag([
    "column",
    "done",
    "name",
    "Done",
    "order",
    "2",
  ]);
  assert.deepEqual(noWip, { id: "done", name: "Done", wip: null, order: 2 });

  assert.equal(parseColumnTag(["column", "x"]), null);
  assert.equal(parseColumnTag(["not-a-column"]), null);
});

test("parseBoard: reads name, owner, ordered columns, shares", () => {
  const board = parseBoard(
    ev(
      {
        kind: 31001,
        created_at: 500,
        content: "## Launch\n\nShip it.",
        pubkey: "ownerpub",
      },
      [
        ["d", "b1"],
        ["name", "Launch"],
        ["p", "ownerpub", "owner"],
        ["column", "done", "name", "Done", "order", "1"],
        ["column", "todo", "name", "To Do", "order", "0"],
        ["column", "blocked", "name", "Blocked", "wip", "2", "order", "2"],
        ["h", "channel-1"],
        ["invite", "guestpub"],
      ],
    ),
  );
  assert.equal(board.name, "Launch");
  assert.equal(board.owner, "ownerpub");
  assert.equal(board.description, "## Launch\n\nShip it.");
  assert.deepEqual(
    board.columns.map((c) => [c.id, c.order, c.wip]),
    [
      ["todo", 0, null],
      ["done", 1, null],
      ["blocked", 2, 2],
    ],
  );
  assert.deepEqual(board.channels, ["channel-1"]);
  assert.deepEqual(board.invites, ["guestpub"]);
});

test("parseCard: reads column/rank/assignees/labels/due + derives title", () => {
  const card = parseCard(
    ev(
      {
        kind: 31002,
        pubkey: "ownerpub",
        content: "## Ship DnD\n\nAdd drag and drop.",
      },
      [
        ["d", "c1"],
        ["a", "31001:ownerpub:b1"],
        ["column", "todo"],
        ["rank", "a1b2"],
        ["p", "assignee1"],
        ["p", "assignee2"],
        ["l", "frontend", "kanban"],
        ["l", "urgent", "kanban"],
        ["due", "2026-09-01"],
      ],
    ),
  );
  assert.equal(card.id, "c1");
  assert.equal(card.boardOwner, "ownerpub");
  assert.equal(card.boardId, "b1");
  assert.equal(card.column, "todo");
  assert.equal(card.rank, "a1b2");
  assert.deepEqual(card.assignees, ["assignee1", "assignee2"]);
  assert.deepEqual(card.labels, ["frontend", "urgent"]);
  assert.equal(card.due, "2026-09-01");
  assert.equal(card.title, "Ship DnD");
});

test("parseCard: missing optionals are null/empty, not misread", () => {
  const card = parseCard(
    ev({ kind: 31002, pubkey: "ownerpub", content: "no heading here" }, [
      ["d", "c2"],
      ["a", "31001:ownerpub:b1"],
      ["column", "todo"],
    ]),
  );
  assert.equal(card.rank, null);
  assert.equal(card.due, null);
  assert.deepEqual(card.assignees, []);
  assert.deepEqual(card.labels, []);
  assert.equal(card.title, "c2"); // fallback to id when no heading
});

test("collapseBoards/collapseCards: latest-created_at head wins per coordinate", () => {
  const older = ev(
    { kind: 31001, id: "old", created_at: 100, pubkey: "owner" },
    [
      ["d", "b1"],
      ["name", "Old name"],
      ["p", "owner", "owner"],
    ],
  );
  const newer = ev(
    { kind: 31001, id: "new", created_at: 200, pubkey: "owner" },
    [
      ["d", "b1"],
      ["name", "New name"],
      ["p", "owner", "owner"],
    ],
  );
  const boards = collapseBoards([older, newer]);
  assert.equal(boards.length, 1);
  assert.equal(boards[0].name, "New name");

  const cardOldEv = ev(
    { kind: 31002, id: "cOld", created_at: 100, pubkey: "owner" },
    [
      ["d", "c1"],
      ["a", "31001:owner:b1"],
      ["column", "todo"],
    ],
  );
  const cardNewEv = ev(
    { kind: 31002, id: "cNew", created_at: 300, pubkey: "owner" },
    [
      ["d", "c1"],
      ["a", "31001:owner:b1"],
      ["column", "done"],
    ],
  );
  const cards = collapseCards([cardOldEv, cardNewEv]);
  assert.equal(cards.length, 1);
  assert.equal(cards[0].column, "done");
});
