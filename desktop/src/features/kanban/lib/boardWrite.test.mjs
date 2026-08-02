import assert from "node:assert/strict";
import test from "node:test";

import { buildBoardEvent, KANBAN_TEMPLATES } from "./boardWrite.ts";

const OWNER =
  "5E869EE84d093c181b415e96695ac1ff0e377c501bb8f6d0880b445e030698dc";

function tags(event) {
  return event.tags;
}

function tagValue(event, name) {
  const found = event.tags.find((tag) => tag[0] === name);
  return found?.[1];
}

function columnTags(event) {
  return event.tags.filter((tag) => tag[0] === "column");
}

test("buildBoardEvent: emits kind 31001 with d/name/p(owner) tags", () => {
  const draft = buildBoardEvent({
    owner: OWNER,
    name: "  Team Board  ",
    template: "kanban",
    description: "Ship things faster.",
  });

  assert.equal(draft.kind, 31001);
  assert.equal(tagValue(draft, "d").length > 0, true);
  assert.match(tagValue(draft, "d"), /^[0-9a-f-]{36}$/u);
  assert.equal(tagValue(draft, "name"), "Team Board"); // trimmed
  // owner is lowercased (canonical source for case-insensitive ownership checks)
  assert.deepEqual(
    tags(draft).find((tag) => tag[0] === "p"),
    ["p", OWNER.toLowerCase(), "owner"],
  );
});

test("buildBoardEvent: kanban template → 4 columns, order 0..3, wip only where set", () => {
  const draft = buildBoardEvent({
    owner: OWNER,
    name: "Team Board",
    template: "kanban",
  });

  assert.equal(KANBAN_TEMPLATES.kanban.length, 4);
  const cols = columnTags(draft);
  assert.equal(cols.length, 4);

  // order is 0-based, contiguous, in template order
  assert.deepEqual(
    cols.map((c) => Number(c[c.indexOf("order") + 1])),
    [0, 1, 2, 3],
  );

  // name field, present on every column in template order
  assert.deepEqual(
    cols.map((c) => c[c.indexOf("name") + 1]),
    ["Backlog", "In Progress", "Review", "Done"],
  );

  // wip present only where bounded (Backlog=5, In Progress=3; Review/Done none)
  const wipByName = Object.fromEntries(
    cols.map((c) => {
      const wipIndex = c.indexOf("wip");
      return [
        c[c.indexOf("name") + 1],
        wipIndex === -1 ? null : Number(c[wipIndex + 1]),
      ];
    }),
  );
  assert.deepEqual(wipByName, {
    Backlog: 5,
    "In Progress": 3,
    Review: null,
    Done: null,
  });
});

test("buildBoardEvent: no h/invite (private by default), content starts ## name", () => {
  const draft = buildBoardEvent({
    owner: OWNER,
    name: "Team Board",
    template: "kanban",
  });

  assert.equal(
    tags(draft).some((tag) => tag[0] === "h"),
    false,
  );
  assert.equal(
    tags(draft).some((tag) => tag[0] === "invite"),
    false,
  );
  assert.equal(draft.content, "## Team Board");

  const withDescription = buildBoardEvent({
    owner: OWNER,
    name: "Team Board",
    description: "  Roadmap  ",
    template: "blank",
  });
  assert.equal(withDescription.content, "## Team Board\n\nRoadmap");
});

test("buildBoardEvent: every generated colid matches ^col-[0-9a-f]{8}$", () => {
  const draft = buildBoardEvent({
    owner: OWNER,
    name: "Team Board",
    template: "blank",
  });
  for (const col of columnTags(draft)) {
    assert.match(col[1], /^col-[0-9a-f]{8}$/u);
  }

  // templates emit distinct colids across all sets (spot-check a multi-column one)
  const sales = buildBoardEvent({
    owner: OWNER,
    name: "Pipeline",
    template: "sales",
  });
  const ids = columnTags(sales).map((c) => c[1]);
  assert.equal(
    new Set(ids).size,
    ids.length,
    "colids must be unique within a board",
  );
});
