import assert from "node:assert/strict";
import test from "node:test";

import {
  projectSections,
  SECTIONS_LANE,
  siblingIds,
} from "./channelSectionsSync.ts";

const DEV = "0".repeat(16);
const reg = (value) => [1, DEV, value];

/** A live section node; `parent` omitted means top level. */
const section = (name, order, parent = null) => ({
  name: reg(name),
  order: reg(order),
  live: reg(true),
  parent: reg(parent),
});

test("a child sorts under its parent and keeps a per-group order", () => {
  const { sections } = projectSections({
    s: {
      work: section("Work", 0),
      clients: section("Clients", 1),
      "work-a": section("Alpha", 7, "work"),
      "work-b": section("Beta", 9, "work"),
    },
  });

  assert.deepEqual(
    sections.map((s) => [s.id, s.order, s.parentId]),
    [
      ["work", 0, undefined],
      ["work-a", 0, "work"],
      ["work-b", 1, "work"],
      ["clients", 1, undefined],
    ],
  );
});

test("nesting stops at two levels: a grandchild stays top-level", () => {
  const { sections } = projectSections({
    s: {
      work: section("Work", 0),
      child: section("Child", 1, "work"),
      grandchild: section("Grandchild", 2, "child"),
    },
  });

  assert.equal(sections.find((s) => s.id === "child")?.parentId, "work");
  assert.equal(
    sections.find((s) => s.id === "grandchild")?.parentId,
    undefined,
  );
});

test("a self-parent is ignored", () => {
  const { sections } = projectSections({
    s: { loop: section("Loop", 0, "loop") },
  });

  assert.deepEqual(sections, [{ id: "loop", name: "Loop", order: 0 }]);
});

test("a self-referential parent still takes children", () => {
  const { sections } = projectSections({
    s: { loop: section("Loop", 0, "loop"), child: section("Child", 1, "loop") },
  });

  assert.deepEqual(
    sections.map((s) => [s.id, s.parentId]),
    [
      ["loop", undefined],
      ["child", "loop"],
    ],
  );
});

test("a two-section cycle degrades to two top-level sections", () => {
  const { sections } = projectSections({
    s: { a: section("A", 0, "b"), b: section("B", 1, "a") },
  });

  assert.deepEqual(
    sections.map((s) => [s.id, s.parentId]),
    [
      ["a", undefined],
      ["b", undefined],
    ],
  );
});

test("deleting a parent promotes its children instead of hiding them", () => {
  const tree = {
    s: {
      work: { ...section("Work", 0), live: reg(false) },
      child: section("Child", 1, "work"),
    },
    a: { "chan-1": reg("child") },
  };
  const { sections, assignments } = projectSections(tree);

  assert.deepEqual(
    sections.map((s) => [s.id, s.parentId]),
    [["child", undefined]],
  );
  // The channel stays in the surviving subsection.
  assert.deepEqual(assignments, { "chan-1": "child" });
});

test("a parent that no longer exists leaves the section top-level", () => {
  const { sections } = projectSections({
    s: { orphan: section("Orphan", 0, "gone") },
  });

  assert.equal(sections[0]?.parentId, undefined);
});

test("channels can be assigned to a subsection", () => {
  const { assignments } = projectSections({
    s: { work: section("Work", 0), child: section("Child", 1, "work") },
    a: { "chan-1": reg("child"), "chan-2": reg("work"), "chan-3": reg("gone") },
  });

  assert.deepEqual(assignments, { "chan-1": "child", "chan-2": "work" });
});

test("siblingIds reads one group at a time", () => {
  const { sections } = projectSections({
    s: {
      work: section("Work", 0),
      clients: section("Clients", 1),
      "work-a": section("Alpha", 2, "work"),
    },
  });

  assert.deepEqual(siblingIds(sections, undefined), ["work", "clients"]);
  assert.deepEqual(siblingIds(sections, "work"), ["work-a"]);
  assert.deepEqual(siblingIds(sections, "clients"), []);
});

test("an invalid parent register is dropped, the section survives", () => {
  const { sections } = projectSections({
    s: {
      work: {
        ...section("Work", 0),
        // `validTree` would strip this; projection must not trust it either.
        parent: reg(42),
      },
    },
  });

  assert.deepEqual(sections, [{ id: "work", name: "Work", order: 0 }]);
});

test("a legacy payload's parentId is imported as a parent register", () => {
  const tree = SECTIONS_LANE.fromLegacy(
    {
      version: 1,
      sections: [
        { id: "work", name: "Work", order: 0 },
        { id: "child", name: "Child", order: 1, parentId: "work" },
      ],
      assignments: { "chan-1": "child" },
    },
    (value) => reg(value),
  );

  assert.deepEqual(projectSections(tree).sections, [
    { id: "work", name: "Work", order: 0 },
    { id: "child", name: "Child", order: 0, parentId: "work" },
  ]);
});

test("the published projection carries parentId for other readers", () => {
  const projection = SECTIONS_LANE.project({
    s: { work: section("Work", 0), child: section("Child", 1, "work") },
  });

  assert.deepEqual(projection.sections, [
    { id: "work", name: "Work", order: 0 },
    { id: "child", name: "Child", order: 0, parentId: "work" },
  ]);
});
