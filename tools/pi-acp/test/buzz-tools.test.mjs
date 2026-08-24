import assert from "node:assert/strict";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createBuzzTools, testOnly } from "../src/buzz-tools.mjs";

const context = {
  channelId: "4dcab690-a2ca-4a56-9e5d-d901d12f83c3",
  triggeringEventIds: ["a".repeat(64)],
  replyTo: "a".repeat(64),
};
const eventId = "b".repeat(64);

async function fixture(t, runCommand) {
  const receiptDir = await fs.mkdtemp(path.join(os.tmpdir(), "pi-acp-tools-"));
  t.after(() => fs.rm(receiptDir, { recursive: true, force: true }));
  const env = {
    BUZZ_PRIVATE_KEY: "nsec_test_only_high_entropy_secret",
    BUZZ_RELAY_URL: "wss://relay.example",
    PI_ACP_RECEIPT_DIR: receiptDir,
    PI_ACP_BUZZ_COMMAND: "/test/buzz",
    PI_ACP_KANBAN_COMMAND: "/test/kanban-ai",
  };
  const tools = createBuzzTools({ getContext: () => context, env, runCommand });
  return {
    env,
    receiptDir,
    reply: tools.find((tool) => tool.name === "buzz_reply"),
    kanban: tools.find((tool) => tool.name === "kanban_tasks"),
  };
}

test("buzz_reply binds routing, connects stdin, and replays one durable receipt", async (t) => {
  const calls = [];
  const f = await fixture(t, async (command, args, options) => {
    calls.push({ command, args, options });
    return {
      code: 0,
      stdout: JSON.stringify({ event_id: eventId, accepted: true }),
      stderr: "",
    };
  });

  const first = await f.reply.execute("call-1", { content: "Visible answer" });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, "/test/buzz");
  assert.deepEqual(calls[0].args, [
    "--format",
    "compact",
    "messages",
    "send",
    "--channel",
    context.channelId,
    "--reply-to",
    context.replyTo,
    "--content",
    "-",
  ]);
  assert.equal(calls[0].options.input, "Visible answer");
  assert.equal(first.details.replay, false);
  assert.equal(first.details.receipt.event_id, eventId);

  const second = await f.reply.execute("call-2", { content: "Visible answer" });
  assert.equal(calls.length, 1, "receipt replay must not publish twice");
  assert.equal(second.details.replay, true);
  assert.equal(second.details.receipt.event_id, eventId);
  await assert.rejects(
    f.reply.execute("call-3", { content: "Different answer" }),
    /different publication is already reserved/,
  );
  assert.equal(
    calls.length,
    1,
    "different content must not bypass the reservation",
  );
});

test("buzz_reply rejects blank content before reserving or running a command", async (t) => {
  let calls = 0;
  const f = await fixture(t, async () => {
    calls += 1;
  });
  await assert.rejects(
    f.reply.execute("call", { content: "  \n" }),
    /must not be blank/,
  );
  assert.equal(calls, 0);
  assert.deepEqual(await fs.readdir(f.receiptDir), []);
});

test("buzz_reply fails closed after an ambiguous reserved publication", async (t) => {
  let calls = 0;
  const f = await fixture(t, async () => {
    calls += 1;
    throw new Error("network result unknown");
  });
  await assert.rejects(
    f.reply.execute("call-1", { content: "answer" }),
    /network result unknown/,
  );
  await assert.rejects(
    f.reply.execute("call-2", { content: "answer" }),
    /already reserved/,
  );
  assert.equal(calls, 1);
});

test("buzz_reply rejects unbound reply targets", async () => {
  const invalid = {
    ...context,
    triggeringEventIds: ["c".repeat(64)],
  };
  const [reply] = createBuzzTools({
    getContext: () => invalid,
    env: { BUZZ_PRIVATE_KEY: "secret" },
    runCommand: async () => assert.fail("must not run"),
  });
  await assert.rejects(
    reply.execute("call", { content: "answer" }),
    /triggering event set is invalid/,
  );
});

test("kanban_tasks emits one bounded filtered compact query", async (t) => {
  const calls = [];
  const f = await fixture(t, async (command, args) => {
    calls.push({ command, args });
    return { code: 0, stdout: '[{"id":"task-1"}]', stderr: "" };
  });
  const result = await f.kanban.execute("call", {
    sprint: 1,
    status: "in-progress",
    channel: "linza-ui-ux",
    search: "report",
    limit: 5,
  });
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0], {
    command: "/test/kanban-ai",
    args: [
      "tasks",
      "--limit",
      "5",
      "--json",
      "--sprint",
      "1",
      "--status",
      "in-progress",
      "--channel",
      "linza-ui-ux",
      "--search",
      "report",
    ],
  });
  assert.equal(result.details.bounded, true);
});

test("reservation keys separate identities without exposing private keys", () => {
  const first = testOnly.reservationKey(context, {
    BUZZ_PRIVATE_KEY: "identity-one",
    BUZZ_RELAY_URL: "wss://relay.example",
  });
  const second = testOnly.reservationKey(context, {
    BUZZ_PRIVATE_KEY: "identity-two",
    BUZZ_RELAY_URL: "wss://relay.example",
  });
  assert.notEqual(first, second);
  assert.doesNotMatch(first, /identity/);
  assert.match(first, /^[0-9a-f]{64}$/);
});
