import assert from "node:assert/strict";
import test from "node:test";
import { stripTerminalSequences, visibleWidth } from "@earendil-works/pi-tui";
import { Picker } from "../src/picker.ts";
import { safeText } from "../src/theme.ts";
import { Transcript } from "../src/transcript.ts";

test("safeText removes terminal and bidi controls", () => {
  const hostile = "ok\x1b]52;c;YQ==\x07\x1b]0;owned\x1b\\\x1b[31mred\u202E\x00";
  assert.equal(safeText(hostile), "okred");
});

test("transcript renders markdown without untrusted terminal sequences", () => {
  const transcript = new Transcript([
    {
      id: "1",
      author: "A\x1b]0;x\x07",
      time: "now",
      content:
        "**bold** [click](javascript:\x1b]52;c;x\x07)\n\n```diff\n+ safe\n```",
    },
  ]);
  const lines = transcript.render(32);
  const plain = lines.map(stripTerminalSequences).join("\n");
  assert.match(plain, /bold/);
  assert.match(plain, /\+ safe/);
  assert.doesNotMatch(lines.join(""), /\]52;|javascript:/);
});

test("renders CJK and emoji within narrow widths", () => {
  const lines = new Transcript([
    {
      id: "1",
      author: "工程师 🐝",
      time: "12:00",
      content: "你好世界 👩🏽‍💻 this-is-a-long-token",
    },
  ]).render(12);
  assert.ok(lines.every((line) => visibleWidth(line) <= 12));
});

test("Markdown entity decoding and reference links cannot introduce terminal controls", () => {
  const rendered = new Transcript([
    {
      id: "entity",
      author: "Agent",
      time: "",
      content:
        "[ref][id]\n\n[id]: javascript:alert(1)\n\n&#27;]52;c;YQ==&#7;\n\n<https://example.test>",
    },
  ])
    .render(80)
    .join("\n");
  assert.ok(!rendered.includes("\x1b]"));
  assert.ok(!rendered.includes("\x07"));
  assert.match(stripTerminalSequences(rendered), /ref/);
});

test("picker filters, navigates, selects, and recovers from zero results", () => {
  let selected = "";
  let renders = 0;
  const picker = new Picker({
    title: "People",
    items: [
      { id: "a", label: "Alice" },
      { id: "b", label: "Bob", detail: "npub-b" },
      { id: "c", label: "Bobby" },
    ],
    onSelect: (item) => {
      selected = item.id;
    },
    onCancel: () => {},
    requestRender: () => {
      renders++;
    },
  });
  picker.focused = true;
  picker.handleInput("b");
  picker.handleInput("\x1b[B");
  picker.handleInput("\r");
  assert.equal(selected, "c");
  assert.ok(renders >= 2);
  picker.handleInput("z");
  assert.match(
    picker.render(24).map(stripTerminalSequences).join("\n"),
    /No matches/,
  );
  picker.handleInput("\x7f");
  assert.doesNotMatch(
    picker.render(24).map(stripTerminalSequences).join("\n"),
    /No matches/,
  );
});

test("live picker updates retain the selected identity rather than its old row index", () => {
  const items = [
    { id: "a", label: "Aurora" },
    { id: "b", label: "Borealis" },
  ];
  let selected = "";
  const picker = new Picker({
    title: "Recipients",
    items: () => items,
    onSelect: (item) => {
      selected = item.id;
    },
    onCancel: () => {},
    requestRender: () => {},
  });
  picker.handleInput("\x1b[B");
  items.unshift({ id: "c", label: "Cygnus" });
  picker.handleInput("\r");
  assert.equal(selected, "b");
});

test("compact picker aligns Unicode rows and keeps selection visible without color", (t) => {
  const noColor = process.env.NO_COLOR;
  process.env.NO_COLOR = "1";
  t.after(() => {
    if (noColor === undefined) delete process.env.NO_COLOR;
    else process.env.NO_COLOR = noColor;
  });
  const picker = new Picker({
    title: "Commands & channels",
    items: [
      { id: "a", label: "工程师 👩🏽‍💻", category: "频道", badge: "current" },
      { id: "b", label: "Switch agent", category: "buzz", badge: "Ctrl+R" },
    ],
    onSelect: () => {},
    onCancel: () => {},
    requestRender: () => {},
  });
  for (const width of [12, 29, 30, 80]) {
    const lines = picker.render(width);
    assert.ok(lines.every((line) => visibleWidth(line) <= width));
    assert.equal(lines.filter((line) => line.startsWith("›")).length, 2);
    assert.equal(lines.length, 5);
    assert.deepEqual(
      lines.slice(3).map(stripTerminalSequences),
      lines.slice(3).map((line) => line.replaceAll("\x1b[0m", "")),
    );
  }
  picker.handleInput("\x1b[B");
  assert.match(picker.render(80)[4], /^›\s+buzz\s+Switch agent\s+Ctrl\+R\s*$/);
});
