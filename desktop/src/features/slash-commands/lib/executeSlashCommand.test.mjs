import assert from "node:assert/strict";
import test from "node:test";

import {
  executeSlashCommand,
  SLASH_COMMAND_ATTACHMENT_ERROR,
  slashCommandChannelOnlyError,
} from "./executeSlashCommand.ts";
import { SlashCommandCancelled } from "./slashCommandCancelled.ts";
import { SlashUsageError } from "./slashUsageError.ts";
import { requestSlashCommandUi } from "./slashCommandUiRequests.ts";
import { buildShrugMessage, SHRUG_MARKDOWN } from "./shrug.ts";

function harness(
  run,
  { hasAttachments = false, isDm = false, context = "any", createContext } = {},
) {
  const failures = [];
  const calls = { run: 0, createContext: 0 };
  const command = {
    name: "demo",
    description: "demo",
    usage: "/demo <thing>",
    context,
    run: async (context) => {
      calls.run += 1;
      return run(context);
    },
  };
  return {
    failures,
    calls,
    execute: () =>
      executeSlashCommand({
        invocation: { command, args: "x" },
        hasAttachments,
        isDm,
        createContext:
          createContext ??
          (async (args) => {
            calls.createContext += 1;
            return { args };
          }),
        reportFailure: (failure) => failures.push(failure),
      }),
  };
}

test("success runs the command once and reports nothing", async () => {
  const h = harness(() => {});
  assert.equal(await h.execute(), "succeeded");
  assert.equal(h.calls.run, 1);
  assert.deepEqual(h.failures, []);
});

test("a usage error shows the usage line and fails", async () => {
  const bare = harness(() => {
    throw new SlashUsageError();
  });
  assert.equal(await bare.execute(), "failed");
  assert.deepEqual(bare.failures, [{ title: "Usage: /demo <thing>" }]);

  const detailed = harness(() => {
    throw new SlashUsageError("Reminders are personal.");
  });
  assert.equal(await detailed.execute(), "failed");
  assert.deepEqual(detailed.failures, [
    { title: "Reminders are personal.", description: "Usage: /demo <thing>" },
  ]);
});

test("a cancelled confirmation is neither a success nor an error", async () => {
  const h = harness(() => {
    throw new SlashCommandCancelled();
  });
  assert.equal(await h.execute(), "cancelled");
  assert.equal(h.calls.run, 1);
  assert.deepEqual(h.failures, []);
});

test("a rejected action surfaces its message and fails", async () => {
  const h = harness(async () => {
    throw new Error("Relay rejected: not an admin");
  });
  assert.equal(await h.execute(), "failed");
  assert.deepEqual(h.failures, [{ title: "Relay rejected: not an admin" }]);

  const opaque = harness(() => {
    throw "boom";
  });
  assert.equal(await opaque.execute(), "failed");
  assert.deepEqual(opaque.failures, [{ title: "Could not run /demo." }]);
});

test("an ambiguous mention fails before the command runs", async () => {
  const h = harness(() => {}, {
    createContext: async () => {
      throw new Error(
        "The mention @sam is ambiguous. Choose a recipient from the mention picker.",
      );
    },
  });
  assert.equal(await h.execute(), "failed");
  assert.equal(h.calls.run, 0);
  assert.match(h.failures[0].title, /ambiguous/);
});

test("attachments reject the command without running it", async () => {
  const h = harness(() => {}, { hasAttachments: true });
  assert.equal(await h.execute(), "failed");
  assert.equal(h.calls.run, 0);
  assert.equal(h.calls.createContext, 0);
  assert.deepEqual(h.failures, [{ title: SLASH_COMMAND_ATTACHMENT_ERROR }]);
});

test("a channel command in a DM is refused without running", async () => {
  const refused = harness(() => {}, { isDm: true, context: "channel" });
  assert.equal(await refused.execute(), "failed");
  assert.equal(refused.calls.run, 0);
  assert.equal(refused.calls.createContext, 0);
  assert.deepEqual(refused.failures, [
    { title: "/demo only works in channels." },
  ]);
  assert.equal(
    slashCommandChannelOnlyError("demo"),
    "/demo only works in channels.",
  );

  const allowed = harness(() => {}, { isDm: true, context: "any" });
  assert.equal(await allowed.execute(), "succeeded");
  const inChannel = harness(() => {}, { isDm: false, context: "channel" });
  assert.equal(await inChannel.execute(), "succeeded");
});

test("UI requests report whether anything handled them", () => {
  assert.equal(requestSlashCommandUi("nobody-listens"), false);
});

test("shrug sends the exact wire bytes", () => {
  const codePoints = (value) =>
    [...value].map((char) => char.codePointAt(0).toString(16));
  assert.deepEqual(codePoints(SHRUG_MARKDOWN), [
    "af",
    "5c",
    "5c",
    "5c",
    "5f",
    "28",
    "30c4",
    "29",
    "5c",
    "5f",
    "2f",
    "af",
  ]);
  assert.equal(buildShrugMessage("hi"), `hi ${SHRUG_MARKDOWN}`);
  assert.equal(buildShrugMessage(""), SHRUG_MARKDOWN);
});

test("shrug appends the escaped glyph and renders back to the plain one", () => {
  assert.equal(buildShrugMessage(""), SHRUG_MARKDOWN);
  assert.equal(buildShrugMessage("  well  "), `well ${SHRUG_MARKDOWN}`);
  const unescaped = SHRUG_MARKDOWN.replace(/\\([\\_])/g, "$1");
  assert.equal(unescaped, "¯\\_(ツ)_/¯");
});
