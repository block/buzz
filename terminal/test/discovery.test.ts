import assert from "node:assert/strict";
import test from "node:test";
import { TerminalApp } from "../src/app.ts";
import { DemoTransport } from "../src/demo.ts";
import { parseLaunch } from "../src/launch.ts";
import type { HostMessage, RelayEvent, Requests } from "../src/protocol.ts";
import { TestTerminal } from "./terminal.ts";

test("directory-only agents arrive in an open picker without losing its search or message draft", async () => {
  const launch = parseLaunch([]);
  let release: () => void = () => {};
  const profiles = new Promise<void>((resolve) => {
    release = resolve;
  });
  const human = "1".repeat(64);
  const atlas = "3".repeat(64);
  const nova = "4".repeat(64);
  const policy = (agent: string, author = human): RelayEvent => ({
    id: agent,
    pubkey: author,
    kind: 30177,
    tags: [["d", agent]],
    created_at: 1,
    content: "{}",
    sig: "",
  });
  class DirectoryRelay extends DemoTransport {
    notify: (message: HostMessage) => void = () => {};
    calls: { method: keyof Requests; params: Requests[keyof Requests] }[] = [];
    override start(receive: (message: HostMessage) => void): void {
      this.notify = receive;
      super.start((message) => {
        if (
          "type" in message &&
          message.type === "event" &&
          [39000, 39002].includes(message.event.kind) &&
          !message.event.tags.some(
            (tag) => tag[0] === "d" && tag[1] === launch.channelId,
          )
        )
          return;
        receive(message);
      });
    }
    override async request<K extends keyof Requests>(
      method: K,
      params: Requests[K],
    ): Promise<unknown> {
      this.calls.push({ method, params });
      if (method === "subscribe") {
        const request = params as Requests["subscribe"];
        if (
          request.subscriptionId === "discovery" &&
          request.filters.some(
            (filter) =>
              filter.kinds.includes(30177) && filter.authors?.includes(human),
          )
        ) {
          for (const event of [
            policy(atlas),
            policy(nova),
            policy("5".repeat(64), "6".repeat(64)),
          ])
            this.notify({ type: "event", subscriptionId: "discovery", event });
        }
        if (request.subscriptionId === "profiles") await profiles;
      }
      return super.request(method, params);
    }
  }
  const terminal = new TestTerminal();
  const relay = new DirectoryRelay();
  const app = new TerminalApp(terminal, relay, () => {}, launch);
  try {
    app.start();
    await terminal.frame();
    assert.equal(app.store.channels.size, 1);
    terminal.input("Please review my changes");
    terminal.input("\x12");
    terminal.input("Nova");
    assert.match(await terminal.frame(), /No matches/);
    release();
    const loaded = await terminal.frame();
    assert.match(loaded, /invite your agent/);
    assert.doesNotMatch(
      loaded.slice(loaded.indexOf("┌"), loaded.indexOf("└")),
      /Atlas/,
    );
    assert.equal(app.store.current?.recipient, atlas);
    assert.equal(app.store.agentPolicies.size, 2);
    terminal.input("\r");
    await terminal.frame();
    assert.equal(app.store.current?.recipient, nova);
    assert.equal(app.editor.getExpandedText(), "Please review my changes");
    terminal.input("\r");
    await terminal.frame();
    const sent = relay.calls.find((call) => call.method === "sendMessage");
    assert.ok(sent);
    assert.deepEqual(
      (sent.params as Requests["sendMessage"]).recipientPubkeys,
      [nova],
    );
    assert.equal(app.store.current?.draft, "");
  } finally {
    release();
    app.stop();
    terminal.screen.dispose();
  }
});
