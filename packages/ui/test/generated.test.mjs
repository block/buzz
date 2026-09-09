import assert from "node:assert/strict";
import { test } from "node:test";
import { parseGeneratedView } from "../src/generated/schema.ts";
const view = (blocks) => ({ version: 1, title: "Example", blocks });
const text = { id: "intro", type: "text", text: "A useful response" };
test("accepts every supported block and keeps strings as data", () => {
  const blocks = [
    text,
    { id: "notice", type: "notice", tone: "success", text: "Done" },
    {
      id: "metric",
      type: "metric",
      label: "Tasks",
      value: "3",
      detail: "Today",
    },
    {
      id: "tasks",
      type: "tasks",
      items: [{ id: "task", label: "Review", status: "running" }],
    },
    {
      id: "table",
      type: "table",
      columns: ["Name"],
      rows: [["<script>alert(1)</script>"]],
    },
    {
      id: "actions",
      type: "actions",
      items: [{ id: "review", label: "Review changes" }],
    },
  ];
  assert.deepEqual(parseGeneratedView(JSON.stringify(view(blocks))), {
    ok: true,
    value: view(blocks),
  });
});
test("rejects invalid structure, executable extensions, and ambiguous identity", () => {
  for (const response of [
    null,
    "{",
    { version: 2, title: "Example", blocks: [text] },
    view([text, text]),
    view([{ ...text, html: "<b>Hi</b>" }]),
    view([{ id: "bad", type: "iframe", src: "https://example.com" }]),
    view([
      {
        id: "a",
        type: "actions",
        items: [{ id: "open", label: "Open", url: "javascript:alert(1)" }],
      },
    ]),
    view([
      { id: "a", type: "actions", items: [{ id: "same", label: "One" }] },
      { id: "b", type: "actions", items: [{ id: "same", label: "Two" }] },
    ]),
    view([
      { id: "t", type: "table", columns: ["Name"], rows: [["one", "two"]] },
    ]),
  ])
    assert.equal(parseGeneratedView(response).ok, false);
});
test("bounds serialized size, block count, text, rows, and actions", () => {
  for (const response of [
    " ".repeat(64001),
    view(Array.from({ length: 21 }, (_, i) => ({ ...text, id: `t${i}` }))),
    view([{ ...text, text: "x".repeat(4001) }]),
    view([
      {
        id: "t",
        type: "table",
        columns: ["Name"],
        rows: Array.from({ length: 51 }, () => ["row"]),
      },
    ]),
    view([
      {
        id: "a",
        type: "actions",
        items: Array.from({ length: 7 }, (_, i) => ({
          id: `a${i}`,
          label: "Action",
        })),
      },
    ]),
  ])
    assert.equal(parseGeneratedView(response).ok, false);
  const cyclic = {};
  cyclic.self = cyclic;
  assert.equal(parseGeneratedView(cyclic).ok, false);
});
