import assert from "node:assert/strict";
import test from "node:test";

import {
  buildChannelAdminCommands,
  canonicalChannelName,
  TOGGLE_CHANNEL_MUTE_UI,
} from "./channelAdminCommands.ts";
import { executeSlashCommand } from "./executeSlashCommand.ts";
import { requestSlashCommandUi } from "./slashCommandUiRequests.ts";

function setup(overrides = {}) {
  const calls = { rename: [], archive: 0, topic: [] };
  const toasts = [];
  const actions = {
    canManageChannel: true,
    canSetTopic: true,
    renameChannel: async (name) => {
      calls.rename.push(name);
    },
    archiveChannel: async () => {
      calls.archive += 1;
    },
    setTopic: async (topic) => {
      calls.topic.push(topic);
    },
    ...overrides,
  };
  const commands = buildChannelAdminCommands(actions);
  const byName = (name) => commands.find((command) => command.name === name);
  const run = async (name, args, context = {}) => {
    const failures = [];
    const result = await executeSlashCommand({
      invocation: { command: byName(name), args },
      hasAttachments: false,
      isDm: false,
      createContext: async () => ({
        channelId: "c1",
        channelName: "general",
        isDm: false,
        args,
        requestUi: requestSlashCommandUi,
        toast: {
          success: (message) => toasts.push(message),
          error: (message) => toasts.push(`error:${message}`),
        },
        ...context,
      }),
      reportFailure: (failure) => failures.push(failure),
    });
    return { result, failures };
  };
  return { calls, toasts, byName, run, commands };
}

test("canonicalChannelName trims and strips leading # characters", () => {
  for (const [input, expected] of [
    ["  new-name ", "new-name"],
    ["#new-name", "new-name"],
    ["##  new-name", "new-name"],
    ["a#b", "a#b"],
    ["   ", ""],
    ["#", ""],
  ]) {
    assert.equal(canonicalChannelName(input), expected, input);
  }
});

test("rename sends the canonical name, and bad input never reaches the relay", async () => {
  const { calls, toasts, run } = setup();

  assert.equal((await run("rename", "#renamed")).result, "succeeded");
  assert.deepEqual(calls.rename, ["renamed"]);
  assert.deepEqual(toasts, ["Renamed to #renamed"]);

  const empty = await run("rename", "  # ");
  assert.equal(empty.result, "failed");
  assert.deepEqual(empty.failures, [{ title: "Usage: /rename <name>" }]);

  const same = await run("rename", "#general");
  assert.equal(same.result, "failed");
  assert.deepEqual(same.failures, [
    { title: "This channel is already named #general" },
  ]);
  assert.deepEqual(calls.rename, ["renamed"]);
});

test("a relay rejection of rename fails with its message and shows no success", async () => {
  const { toasts, run } = setup({
    renameChannel: async () => {
      throw new Error("name already taken");
    },
  });
  const { result, failures } = await run("rename", "taken");
  assert.equal(result, "failed");
  assert.deepEqual(failures, [{ title: "name already taken" }]);
  assert.deepEqual(toasts, []);
});

test("archive and topic run through their actions", async () => {
  const { calls, toasts, run } = setup();
  assert.equal((await run("archive", "")).result, "succeeded");
  assert.equal(calls.archive, 1);
  assert.equal((await run("topic", "  Ship it  ")).result, "succeeded");
  assert.deepEqual(calls.topic, ["Ship it"]);
  assert.deepEqual(toasts, ["Archived #general", "Topic set"]);

  const bare = await run("topic", "   ");
  assert.equal(bare.result, "failed");
  assert.deepEqual(bare.failures, [{ title: "Usage: /topic <text>" }]);
  assert.deepEqual(calls.topic, ["Ship it"]);
});

test("visibility follows the manage and post capabilities", () => {
  const visible = (actions) =>
    setup(actions)
      .commands.filter((command) => command.isVisible?.() ?? true)
      .map((command) => command.name)
      .sort();

  assert.deepEqual(visible({}), ["archive", "mute", "rename", "topic"]);
  assert.deepEqual(visible({ canManageChannel: false }), ["mute", "topic"]);
  assert.deepEqual(visible({ canSetTopic: false }), [
    "archive",
    "mute",
    "rename",
  ]);
});

test("mute toasts the state the owning handler applied, and fails when it cannot", async () => {
  const { toasts, run } = setup();
  const handler = (muted) => (name, payload) => {
    assert.equal(name, TOGGLE_CHANNEL_MUTE_UI);
    assert.equal(payload.channelId, "c1");
    if (muted !== null) payload.report(muted);
    return true;
  };

  assert.equal(
    (await run("mute", "", { requestUi: handler(true) })).result,
    "succeeded",
  );
  assert.equal(
    (await run("mute", "", { requestUi: handler(false) })).result,
    "succeeded",
  );
  assert.equal(
    (
      await run("mute", "", {
        requestUi: handler(true),
        isDm: true,
        channelName: "alice",
      })
    ).result,
    "succeeded",
  );
  assert.deepEqual(toasts, [
    "Muted #general",
    "Unmuted #general",
    "Muted alice",
  ]);

  const failure = [{ title: "Couldn't change the mute setting. Try again." }];
  const unhandled = await run("mute", "");
  assert.equal(unhandled.result, "failed");
  assert.deepEqual(unhandled.failures, failure);
  const silent = await run("mute", "", { requestUi: handler(null) });
  assert.equal(silent.result, "failed");
  assert.deepEqual(silent.failures, failure);
  assert.equal(toasts.length, 3);
});
