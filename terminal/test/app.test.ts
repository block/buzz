import assert from "node:assert/strict";
import test from "node:test";
import type { Terminal } from "@earendil-works/pi-tui";
import xterm from "@xterm/headless";
import { TerminalApp } from "../src/app.ts";
import { DemoTransport } from "../src/demo.ts";
import { parseLaunch } from "../src/launch.ts";
import type { Requests } from "../src/protocol.ts";

class TestTerminal implements Terminal {
  columns = 100;
  rows = 32;
  kittyProtocolActive = false;
  screen = new xterm.Terminal({ cols: 100, rows: 32, allowProposedApi: true });
  input: (data: string) => void = () => {};
  resize: () => void = () => {};
  start(input: (data: string) => void, resize: () => void): void {
    this.input = input;
    this.resize = resize;
  }
  stop(): void {}
  async drainInput(): Promise<void> {}
  write(data: string): void {
    this.screen.write(data);
  }
  moveBy(): void {}
  hideCursor(): void {}
  showCursor(): void {}
  clearLine(): void {}
  clearFromCursor(): void {}
  clearScreen(): void {}
  setTitle(): void {}
  setProgress(): void {}
  async frame(): Promise<string> {
    await new Promise((resolve) => setTimeout(resolve, 40));
    await new Promise<void>((resolve) => this.screen.write("", resolve));
    const buffer = this.screen.buffer.active;
    return Array.from(
      { length: this.rows },
      (_, row) =>
        buffer.getLine(row + buffer.viewportY)?.translateToString(true) ?? "",
    ).join("\n");
  }
}

test("real TUI keyboard path preserves multiline drafts and routes a send across contexts", async () => {
  const terminal = new TestTerminal();
  const app = new TerminalApp(terminal, new DemoTransport(), () => {});
  try {
    app.start();
    assert.match(await terminal.frame(), /Your agents keep working/);
    terminal.input("\x0b");
    assert.match(await terminal.frame(), /#engineering/);
    terminal.input("\r");
    await terminal.frame();
    terminal.input("\x12");
    assert.match(await terminal.frame(), /Atlas/);
    terminal.input("\r");
    await terminal.frame();
    terminal.input("draft A");
    terminal.input("\n");
    terminal.input("second line");
    assert.equal(app.store.current?.draft, "draft A\nsecond line");
    terminal.input("\x0b");
    await terminal.frame();
    terminal.input("platform");
    terminal.input("\r");
    await terminal.frame();
    terminal.input("\x12");
    await terminal.frame();
    terminal.input("\r");
    await terminal.frame();
    terminal.input("message B");
    terminal.input("\r");
    assert.match(await terminal.frame(), /message B/);
    assert.equal(app.store.current?.draft, "");
    assert.equal(app.store.current?.recipient, "4".repeat(64));
    terminal.input("\x0b");
    await terminal.frame();
    terminal.input("engineering");
    terminal.input("\r");
    const screen = await terminal.frame();
    assert.match(screen, /draft A/);
    assert.match(screen, /second line/);
    assert.equal(app.store.current?.recipient, "3".repeat(64));
    terminal.input("\x1ba");
    assert.match(await terminal.frame(), /Agent activity/);
    terminal.input("\x1b");
    await terminal.frame();
    assert.equal(app.editor.getExpandedText(), "draft A\nsecond line");
    terminal.columns = 42;
    terminal.rows = 18;
    terminal.screen.resize(42, 18);
    terminal.resize();
    const narrow = await terminal.frame();
    assert.match(narrow, /draft A/);
    assert.match(narrow, /\^G commands/);
  } finally {
    app.stop();
    terminal.screen.dispose();
  }
});

test("typing during channel creation survives activation and sends to the configured agent", async () => {
  let release: () => void = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  class DelayedRelay extends DemoTransport {
    override async request<K extends keyof Requests>(
      method: K,
      params: Requests[K],
    ): Promise<unknown> {
      if (method === "createChannel") await gate;
      return super.request(method, params);
    }
  }
  const terminal = new TestTerminal();
  const launch = parseLaunch([], "3".repeat(64));
  const app = new TerminalApp(terminal, new DelayedRelay(), () => {}, launch);
  try {
    app.start();
    assert.match(await terminal.frame(), /Creating a private channel/);
    terminal.input("Please review the startup flow");
    terminal.input("\n");
    terminal.input("Keep my draft intact.");
    release();
    const screen = await terminal.frame();
    assert.match(screen, /To Atlas/);
    assert.match(screen, /Keep my draft intact/);
    assert.equal(app.store.current?.channelId, launch.channelId);
    assert.equal(
      app.store.current?.draft,
      "Please review the startup flow\nKeep my draft intact.",
    );
    terminal.input("\r");
    assert.match(await terminal.frame(), /Please review the startup flow/);
    assert.equal(app.store.current?.draft, "");
  } finally {
    app.stop();
    terminal.screen.dispose();
  }
});
