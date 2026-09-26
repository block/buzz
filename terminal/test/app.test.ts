import assert from "node:assert/strict";
import test from "node:test";
import { TerminalApp } from "../src/app.ts";
import { DemoTransport } from "../src/demo.ts";
import { parseLaunch } from "../src/launch.ts";
import type { Requests } from "../src/protocol.ts";
import { TestTerminal } from "./terminal.ts";

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

test("typing during channel creation survives activation and sends to the default agent without a picker", async () => {
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
  const launch = parseLaunch([]);
  const app = new TerminalApp(terminal, new DelayedRelay(), () => {}, launch);
  try {
    app.start();
    assert.match(await terminal.frame(), /Creating a private channel/);
    terminal.input("Please review the startup flow");
    terminal.input("\n");
    terminal.input("Keep my draft intact.");
    terminal.input("\x1ba");
    assert.match(await terminal.frame(), /Agent activity/);
    assert.equal(app.editor.getExpandedText(), "");
    terminal.input("/status");
    terminal.input("\x1b");
    await terminal.frame();
    assert.equal(
      app.editor.getExpandedText(),
      "Please review the startup flow\nKeep my draft intact.",
    );
    terminal.input("\x1ba");
    await terminal.frame();
    release();
    assert.match(await terminal.frame(), /Agent activity/);
    assert.equal(app.editor.getExpandedText(), "/status");
    terminal.input("\x1b");
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
