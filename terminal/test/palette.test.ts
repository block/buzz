import assert from "node:assert/strict";
import test from "node:test";
import { TerminalApp } from "../src/app.ts";
import { DemoTransport } from "../src/demo.ts";
import { parseLaunch } from "../src/launch.ts";
import { TestTerminal } from "./terminal.ts";

const engineering = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const platform = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

test("slash and both navigation shortcuts share live channel and command search without consuming drafts", async () => {
  const terminal = new TestTerminal();
  let exited = false;
  const app = new TerminalApp(
    terminal,
    new DemoTransport(),
    () => {
      exited = true;
    },
    parseLaunch(["join", engineering]),
  );
  try {
    app.start();
    await terminal.frame();
    terminal.input("/");
    let frame = await terminal.frame();
    assert.equal(app.tui.hasOverlay(), true);
    assert.match(frame, /Commands & channels/);
    assert.match(frame, /#platform/);
    assert.match(frame, /Switch agent\s+Ctrl\+R/);
    assert.equal(frame.match(/CHANNELS/g)?.length, 1);
    assert.equal(frame.match(/ACTIONS/g)?.length, 1);
    assert.match(frame, /Agent activity\s+Alt\+A/);
    assert.match(frame, /Reconnect/);
    assert.match(frame, /1–6 of 15/);
    assert.equal(app.editor.getExpandedText(), "");
    terminal.input("/activity");
    frame = await terminal.frame();
    assert.match(frame, /Agent activity\s+Alt\+A/);
    assert.doesNotMatch(frame, /#platform/);
    terminal.input("\r");
    assert.match(await terminal.frame(), /Agent activity/);
    terminal.input("\x1b");
    await terminal.frame();
    terminal.input("draft");
    terminal.input("/");
    assert.equal(app.tui.hasOverlay(), false);
    assert.equal(app.editor.getExpandedText(), "draft/");
    terminal.input("\x07");
    await terminal.frame();
    terminal.input("platform");
    terminal.input("\r");
    await terminal.frame();
    assert.equal(app.store.current?.channelId, platform);
    terminal.input("\x0b");
    assert.match(await terminal.frame(), /Commands & channels/);
    terminal.input("engineering");
    terminal.input("\r");
    await terminal.frame();
    assert.equal(app.editor.getExpandedText(), "draft/");
    terminal.input("\x07");
    terminal.input("no-such-channel-or-command");
    assert.match(await terminal.frame(), /No matches/);
    terminal.input("\x1b");
    await terminal.frame();
    assert.equal(app.editor.getExpandedText(), "draft/");
    app.editor.setText("");
    terminal.input("\x1b[200~/quit\x1b[201~");
    assert.equal(app.tui.hasOverlay(), false);
    terminal.input("\r");
    await terminal.frame();
    assert.equal(exited, false);
    assert.ok(app.store.current);
    assert.ok(
      app.store
        .events(app.store.current)
        .some((event) => event.content === "/quit"),
    );
  } finally {
    app.stop();
    terminal.screen.dispose();
  }
});

test("agent color matches chat, the palette dims only its backdrop, and closing restores the header", async (t) => {
  const noColor = process.env.NO_COLOR;
  delete process.env.NO_COLOR;
  t.after(() => {
    if (noColor === undefined) delete process.env.NO_COLOR;
    else process.env.NO_COLOR = noColor;
  });
  const terminal = new TestTerminal();
  const app = new TerminalApp(
    terminal,
    new DemoTransport(),
    () => {},
    parseLaunch(["join", engineering]),
  );
  const cell = (row: number, column: number) => {
    const buffer = terminal.screen.buffer.active;
    const value = buffer.getLine(buffer.viewportY + row)?.getCell(column);
    assert.ok(value);
    return value;
  };
  try {
    app.start();
    const frame = await terminal.frame();
    const rows = frame.split("\n");
    const headerColumn = rows[0].indexOf("Atlas");
    assert.ok(headerColumn > terminal.columns / 2);
    assert.match(rows[0], /Atlas\s*$/);
    const messageRow = rows.findIndex(
      (line, index) => index > 0 && line.startsWith("Atlas  "),
    );
    assert.ok(messageRow > 0);
    const color = cell(0, headerColumn).getFgColor();
    assert.ok(color > 15);
    assert.equal(cell(messageRow, 0).getFgColor(), color);
    assert.equal(cell(0, headerColumn).isDim(), 0);
    assert.doesNotMatch(frame, /\^K contexts|\^R recipient|\^G commands/);
    terminal.input("/");
    let palette = await terminal.frame();
    const titleRow = palette
      .split("\n")
      .findIndex((line) => line.includes("┌ Commands & channels"));
    assert.ok(titleRow > 0);
    const titleColumn = palette.split("\n")[titleRow].indexOf("Commands");
    assert.notEqual(cell(0, headerColumn).isDim(), 0);
    assert.equal(cell(0, headerColumn).getFgColor(), 240);
    assert.equal(cell(titleRow, titleColumn).isDim(), 0);
    const paletteRows = palette.split("\n");
    const selectedRow = paletteRows.findIndex(
      (line, index) => index > titleRow && line.includes("#engineering"),
    );
    assert.ok(selectedRow > titleRow);
    const left = paletteRows[titleRow].indexOf("┌") + 2;
    const right = paletteRows[selectedRow].lastIndexOf("│") - 2;
    assert.equal(left - 2, 18);
    assert.equal(right + 2, 81);
    const bottom = paletteRows.findIndex((line) => line[left - 2] === "└");
    assert.ok(bottom > selectedRow);
    for (let row = titleRow + 1; row < bottom; row++) {
      assert.equal(paletteRows[row][left - 2], "│");
      assert.equal(paletteRows[row][right + 2], "│");
    }
    for (let column = left; column <= right; column++) {
      assert.equal(cell(selectedRow, column).isBgDefault(), true);
      assert.equal(cell(selectedRow, column).getFgColor(), 3);
      assert.equal(cell(selectedRow, column).isDim(), 0);
    }
    assert.equal(cell(selectedRow + 1, left).isBgDefault(), true);
    assert.equal(paletteRows[selectedRow + 1].includes("#platform"), true);
    terminal.input("\x1b[B");
    await terminal.frame();
    assert.equal(cell(selectedRow, left).isBgDefault(), true);
    assert.notEqual(cell(selectedRow, left).getFgColor(), 3);
    assert.equal(cell(selectedRow + 1, left).getFgColor(), 3);
    terminal.columns = 42;
    terminal.rows = 18;
    terminal.screen.resize(42, 18);
    terminal.resize();
    palette = await terminal.frame();
    assert.match(palette, /Commands & channels/);
    assert.match(palette, /Enter · Esc/);
    assert.match(palette, /CHANNELS/);
    assert.match(palette, /ACTIONS/);
    const narrowTitle = palette
      .split("\n")
      .find((line) => line.includes("┌ Commands"));
    assert.equal(narrowTitle?.indexOf("┌"), 2);
    assert.equal(narrowTitle?.indexOf("┐"), 39);
    terminal.input("/activity");
    assert.match(await terminal.frame(), /Agent activity\s+Alt\+A/);
    terminal.input("\x1b");
    const restored = (await terminal.frame()).split("\n")[0];
    assert.match(restored, /Atlas\s*$/);
    assert.equal(cell(0, restored.indexOf("Atlas")).getFgColor(), color);
    assert.equal(cell(0, restored.indexOf("Atlas")).isDim(), 0);
    assert.equal(app.editor.getExpandedText(), "");
  } finally {
    app.stop();
    terminal.screen.dispose();
  }
});

test("the quiet footer retains notices and help exposes the current channel's resume command", async () => {
  const terminal = new TestTerminal();
  const app = new TerminalApp(
    terminal,
    new DemoTransport(),
    () => {},
    parseLaunch(["join", engineering]),
  );
  try {
    app.start();
    let frame = await terminal.frame();
    assert.match(
      frame.split("\n").at(-1) ?? "",
      /Commands & channels\s+DEMO \/ OFFLINE/,
    );
    assert.doesNotMatch(frame, /Resume with buzz join/);
    app.store.handle({
      type: "notice",
      message: "Delivery blocked: reconnect to retry",
    });
    frame = await terminal.frame();
    assert.match(
      frame.split("\n").at(-1) ?? "",
      /Delivery blocked: reconnect to retry/,
    );
    app.store.handle({ type: "connection", status: "connected" });
    frame = await terminal.frame();
    assert.match(frame.split("\n").at(-1) ?? "", /Commands & channels/);
    assert.doesNotMatch(frame, /Delivery blocked/);
    app.store.open(platform);
    terminal.input("/");
    terminal.input("/help");
    terminal.input("\r");
    frame = await terminal.frame();
    assert.match(frame, /Resume this channel/);
    assert.ok(frame.includes(`buzz join ${platform}`));
    assert.ok(!frame.includes(`buzz join ${engineering}`));
  } finally {
    app.stop();
    terminal.screen.dispose();
  }
});
