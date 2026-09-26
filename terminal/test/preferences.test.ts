import assert from "node:assert/strict";
import {
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { TerminalApp } from "../src/app.ts";
import { DemoTransport } from "../src/demo.ts";
import { parseLaunch } from "../src/launch.ts";
import { AgentPreferences } from "../src/preferences.ts";
import type { HostMessage } from "../src/protocol.ts";
import { Session } from "../src/session.ts";
import { Store } from "../src/store.ts";
import { TestTerminal } from "./terminal.ts";

const human = "1".repeat(64);
const atlas = "3".repeat(64);
const nova = "4".repeat(64);
const relay = "demo://local";
const engineering = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

test("Ctrl+R selection persists to disk and the next new channel sends without a picker", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "buzz-preferences-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  for (const run of [0, 1]) {
    const terminal = new TestTerminal();
    const app = new TerminalApp(
      terminal,
      new DemoTransport(),
      () => {},
      parseLaunch([]),
      new AgentPreferences(directory),
    );
    try {
      app.start();
      await terminal.frame();
      if (run === 0) {
        assert.equal(app.store.current?.recipient, atlas);
        terminal.input("Keep my draft");
        terminal.input("\x12");
        terminal.input("Nova");
        await terminal.frame();
        terminal.input("\r");
        await terminal.frame();
        assert.equal(app.editor.getExpandedText(), "Keep my draft");
      } else {
        assert.match((await terminal.frame()).split("\n")[0], /Nova\s*$/);
        assert.equal(app.tui.hasOverlay(), false);
        terminal.input("Use my remembered agent");
      }
      assert.equal(app.store.current?.recipient, nova);
      terminal.input("\r");
      await terminal.frame();
      const view = app.store.current;
      assert.ok(view);
      assert.equal(view.draft, "");
      assert.ok(
        app.store
          .events(view)
          .some((event) =>
            event.tags.some((tag) => tag[0] === "p" && tag[1] === nova),
          ),
      );
    } finally {
      app.stop();
      terminal.screen.dispose();
    }
  }
  const preferences = new AgentPreferences(directory);
  assert.equal(preferences.read(relay, human), nova);
  assert.equal(preferences.read("demo://other", human), undefined);
  assert.equal(preferences.read(relay, "2".repeat(64)), undefined);
  const files = readdirSync(directory);
  assert.equal(files.length, 1);
  assert.equal(readFileSync(join(directory, files[0]), "utf8"), `${nova}\n`);
  writeFileSync(join(directory, files[0]), "invalid");
  assert.equal(preferences.read(relay, human), undefined);
});

test("defaults honor overrides and verified ownership without overwriting the last manual choice", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "buzz-precedence-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  class NamedRelay extends DemoTransport {
    unowned?: string;
    override start(receive: (message: HostMessage) => void): void {
      super.start((message) => {
        if (
          "type" in message &&
          message.type === "event" &&
          message.event.kind === 0
        ) {
          message.event.content = JSON.stringify({
            name: message.event.pubkey === atlas ? "Zulu" : "Alpha",
          });
          if (message.event.pubkey === this.unowned)
            message.ownerPubkey = "2".repeat(64);
        }
        receive(message);
      });
    }
  }
  for (const scenario of [
    { saved: "5".repeat(64), expected: nova },
    { saved: human, expected: nova },
    { saved: atlas, expected: atlas },
    { saved: atlas, unowned: atlas, expected: nova },
    { saved: nova, explicit: atlas, expected: atlas },
    { saved: nova, join: true, expected: atlas },
  ]) {
    const preferences = new AgentPreferences(directory);
    preferences.write(relay, human, scenario.saved);
    const store = new Store(preferences);
    const wire = new NamedRelay();
    wire.unowned = scenario.unowned;
    const session = new Session(
      store,
      wire,
      parseLaunch(
        scenario.join ? ["join", engineering] : [],
        scenario.explicit,
      ),
    );
    try {
      session.start();
      await new Promise<void>((resolve) => setImmediate(resolve));
      assert.equal(
        store.current?.recipient,
        scenario.expected,
        JSON.stringify(scenario),
      );
      assert.equal(preferences.read(relay, human), scenario.saved);
    } finally {
      session.close();
    }
  }
});

test("failed preference writes retain the exact choice for setup retry without disconnecting", async (t) => {
  const directory = mkdtempSync(join(tmpdir(), "buzz-retry-preference-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  class FailingPreferences extends AgentPreferences {
    fail = true;
    override write(relay: string, human: string, agent: string): void {
      if (this.fail) throw new Error("disk full");
      super.write(relay, human, agent);
    }
  }
  const preferences = new FailingPreferences(directory);
  const store = new Store(preferences);
  const session = new Session(store, new DemoTransport(), parseLaunch([]));
  try {
    session.start();
    await new Promise<void>((resolve) => setImmediate(resolve));
    const view = store.current;
    assert.ok(view && session.startup);
    await session.startup.selectAgent(view, nova);
    assert.equal(view.recipient, undefined);
    assert.match(store.notice, /disk full/);
    assert.equal(store.connection, "connected");
    assert.equal(preferences.read(relay, human), undefined);
    preferences.fail = false;
    await session.startup.retry();
    assert.equal(view.recipient, nova);
    assert.equal(preferences.read(relay, human), nova);
  } finally {
    session.close();
  }
});
