import assert from "node:assert/strict";
import { TerminalApp } from "../src/app.ts";
import { parseLaunch } from "../src/launch.ts";
import { HostTransport } from "../src/transport.ts";
import { TestTerminal } from "./terminal.ts";

const key = process.env.BUZZ_PRIVATE_KEY;
const agent = process.env.BUZZ_TEST_AGENT;
const host = process.env.BUZZ_TERMINAL_HOST;
assert.ok(key && agent && host);
const launch = parseLaunch([]);
const terminal = new TestTerminal();
const app = new TerminalApp(
  terminal,
  new HostTransport(host),
  () => {},
  launch,
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
  await waitFor(() => !!app.store.current, "new channel opens");
  terminal.input("\x12");
  terminal.input("Borealis");
  await waitFor(
    () => app.store.profiles.has(agent),
    "directory-only agent loads",
  );
  assert.match(await terminal.frame(), /Borealis/);
  assert.match(await terminal.frame(), /invite your agent/);
  assert.equal(app.store.current?.recipient, undefined);
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

process.env.BUZZ_PRIVATE_KEY = key;
const resumedTerminal = new TestTerminal();
const resumed = new TerminalApp(
  resumedTerminal,
  new HostTransport(host),
  () => {},
  parseLaunch(["join", launch.channelId]),
);
try {
  resumed.start();
  const deadline = Date.now() + 15_000;
  while (resumed.store.current?.history !== "ready") {
    assert.ok(Date.now() < deadline, `join: ${resumed.store.notice}`);
    await resumedTerminal.frame();
  }
  assert.equal(resumed.store.current.channelId, launch.channelId);
  assert.ok(
    resumed.store
      .events(resumed.store.current)
      .some(
        (event) => event.content === "Reply to the terminal integration probe",
      ),
  );
  process.stdout.write(
    "PASS: buzz join restored the same channel and message history\n",
  );
} finally {
  resumed.stop();
  resumedTerminal.screen.dispose();
}
