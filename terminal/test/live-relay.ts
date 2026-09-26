import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { TerminalApp } from "../src/app.ts";
import { parseLaunch } from "../src/launch.ts";
import { AgentPreferences } from "../src/preferences.ts";
import { HostTransport } from "../src/transport.ts";
import { TestTerminal } from "./terminal.ts";

const key = process.env.BUZZ_PRIVATE_KEY;
const agent = process.env.BUZZ_TEST_AGENT;
const host = process.env.BUZZ_TERMINAL_HOST;
assert.ok(key && agent && host);
const directory = mkdtempSync(join(tmpdir(), "buzz-live-preferences-"));
process.once("exit", () => rmSync(directory, { recursive: true, force: true }));
const launch = parseLaunch([]);
const terminal = new TestTerminal();
const app = new TerminalApp(
  terminal,
  new HostTransport(host),
  () => {},
  launch,
  new AgentPreferences(directory),
);

async function waitFor(check: () => boolean, label: string): Promise<void> {
  const deadline = Date.now() + 15_000;
  while (!check()) {
    if (Date.now() >= deadline)
      assert.fail(`${label}: ${app.store.notice}\n${await terminal.frame()}`);
    await terminal.frame();
  }
}

try {
  app.start();
  await waitFor(
    () => !!app.store.current?.recipient,
    "first agent selected automatically",
  );
  assert.ok(app.store.current?.recipient);
  assert.equal(app.store.name(app.store.current.recipient), "Aurora");
  assert.equal(app.tui.hasOverlay(), false);
  terminal.input("\x12");
  terminal.input("Borealis");
  await waitFor(
    () => app.store.profiles.has(agent),
    "directory-only agent loads",
  );
  assert.match(await terminal.frame(), /Borealis/);
  assert.match(await terminal.frame(), /invite your agent/);
  terminal.input("\r");
  await waitFor(
    () => app.store.current?.recipient === agent,
    "invitation confirmed",
  );
  terminal.input("Reply to the terminal integration probe");
  terminal.input("\r");
  await waitFor(
    () =>
      !!app.store.current &&
      app.store
        .events(app.store.current)
        .some(
          (event) =>
            event.content === "Reply to the terminal integration probe",
        ),
    "signed message accepted",
  );
  terminal.input("\x14");
  await terminal.frame();
  terminal.input("\r");
  await waitFor(
    () =>
      !!app.store.current &&
      app.store
        .events(app.store.current)
        .some(
          (event) =>
            event.pubkey === agent &&
            event.content === "Probe received by Borealis",
        ),
    "agent reply in thread",
  );
  assert.match(await terminal.frame(), /Probe received by Borealis/);
  process.stdout.write(
    "PASS: real TUI discovered, invited, sent, and rendered agent reply\n",
  );
} finally {
  app.stop();
  terminal.screen.dispose();
}

for (const next of [parseLaunch(["join", launch.channelId]), parseLaunch([])]) {
  process.env.BUZZ_PRIVATE_KEY = key;
  const resumedTerminal = new TestTerminal();
  const resumed: TerminalApp = new TerminalApp(
    resumedTerminal,
    new HostTransport(host),
    () => {},
    next,
    new AgentPreferences(directory),
  );
  try {
    resumed.start();
    const deadline = Date.now() + 15_000;
    while (
      resumed.store.current?.history !== "ready" ||
      resumed.store.current.recipient !== agent
    ) {
      assert.ok(Date.now() < deadline, `join: ${resumed.store.notice}`);
      await resumedTerminal.frame();
    }
    assert.equal(resumed.store.current.channelId, next.channelId);
    assert.equal(resumed.tui.hasOverlay(), false);
    if (next.mode === "join")
      assert.ok(
        resumed.store
          .events(resumed.store.current)
          .some(
            (event) =>
              event.content === "Reply to the terminal integration probe",
          ),
      );
    process.stdout.write(
      `PASS: ${next.mode} restored the remembered agent without a picker${next.mode === "join" ? " and message history" : " in a fresh channel"}\n`,
    );
  } finally {
    resumed.stop();
    resumedTerminal.screen.dispose();
  }
}
