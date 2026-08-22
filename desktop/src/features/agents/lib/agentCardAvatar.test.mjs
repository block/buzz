import assert from "node:assert/strict";
import test from "node:test";

import {
  isAgentCardAvatarLoading,
  resolveAgentCardAvatarUrl,
  resolveCurrentRuntimeAvatarUrl,
} from "./agentCardAvatar.ts";

const previousStockAvatarUrl = "https://catalog.example/runtime-a.png";
const currentStockAvatarUrl = "https://catalog.example/runtime-b.png";
const stockAvatarUrls = new Set([
  previousStockAvatarUrl,
  currentStockAvatarUrl,
]);

const runtimes = [
  {
    id: "runtime-a",
    command: "runtime-a-command",
    binaryPath: "/usr/local/bin/runtime-a",
    avatarUrl: previousStockAvatarUrl,
  },
  {
    id: "runtime-b",
    command: "runtime-b-command",
    binaryPath: "/usr/local/bin/runtime-b",
    avatarUrl: currentStockAvatarUrl,
  },
  {
    id: "unavailable-runtime",
    command: null,
    binaryPath: null,
    avatarUrl: "https://catalog.example/unavailable.png",
  },
  {
    id: "runtime-without-avatar",
    command: "runtime-without-avatar-command",
    binaryPath: null,
    avatarUrl: "",
  },
];

test("current runtime avatar resolves stored runtime id before command aliases", () => {
  assert.equal(
    resolveCurrentRuntimeAvatarUrl(
      {
        agentCommand: "/different/path/runtime-b-alias",
        agentCommandOverride: null,
        runtime: "runtime-b",
      },
      { runtime: "runtime-a" },
      runtimes,
    ),
    currentStockAvatarUrl,
  );
});

test("current runtime avatar falls back to catalog command and executable path", () => {
  for (const agentCommand of [
    "runtime-b-command",
    "/usr/local/bin/runtime-b",
  ]) {
    assert.equal(
      resolveCurrentRuntimeAvatarUrl(
        { agentCommand, agentCommandOverride: agentCommand, runtime: null },
        { runtime: "runtime-a" },
        runtimes,
      ),
      currentStockAvatarUrl,
    );
  }
});

test("current runtime avatar falls back to an inherited persona runtime", () => {
  assert.equal(
    resolveCurrentRuntimeAvatarUrl(
      {
        agentCommand: "missing-command",
        agentCommandOverride: null,
        runtime: null,
      },
      { runtime: "runtime-b" },
      runtimes,
    ),
    currentStockAvatarUrl,
  );
});

test("current runtime avatar does not use persona fallback for an explicit override", () => {
  assert.equal(
    resolveCurrentRuntimeAvatarUrl(
      {
        agentCommand: "custom-command",
        agentCommandOverride: "custom-command",
        runtime: null,
      },
      { runtime: "runtime-b" },
      runtimes,
    ),
    null,
  );
});

test("current runtime avatar resolves an unavailable command through its runtime id", () => {
  assert.equal(
    resolveCurrentRuntimeAvatarUrl(
      {
        agentCommand: "unavailable-runtime-command",
        agentCommandOverride: null,
        runtime: "unavailable-runtime",
      },
      { runtime: null },
      runtimes,
    ),
    "https://catalog.example/unavailable.png",
  );
});

test("running agent card clears stale stock for a runtime without an avatar", () => {
  const runtimeAvatarUrl = resolveCurrentRuntimeAvatarUrl(
    {
      agentCommand: "runtime-without-avatar-command",
      agentCommandOverride: "runtime-without-avatar-command",
      runtime: "runtime-a",
    },
    { runtime: null },
    runtimes,
  );

  assert.equal(runtimeAvatarUrl, null);
  assert.equal(
    resolveAgentCardAvatarUrl(
      previousStockAvatarUrl,
      null,
      runtimeAvatarUrl,
      stockAvatarUrls,
    ),
    null,
  );
});

test("running agent card prefers the pubkey profile avatar", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      "https://relay.example/instance.png",
      "https://relay.example/definition.png",
    ),
    "https://relay.example/instance.png",
  );
});

test("running agent card falls back to the definition avatar", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(null, " https://relay.example/definition.png "),
    "https://relay.example/definition.png",
  );
});

test("running agent card keeps a matching stock avatar", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      currentStockAvatarUrl,
      null,
      currentStockAvatarUrl,
      stockAvatarUrls,
    ),
    currentStockAvatarUrl,
  );
});

test("running agent card replaces a stale stock avatar after a runtime transition", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      previousStockAvatarUrl,
      null,
      currentStockAvatarUrl,
      stockAvatarUrls,
    ),
    currentStockAvatarUrl,
  );
});

test("running agent card preserves a custom profile avatar", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      " https://profiles.example/custom.png ",
      previousStockAvatarUrl,
      currentStockAvatarUrl,
      stockAvatarUrls,
    ),
    "https://profiles.example/custom.png",
  );
});

test("running agent card replaces a stock definition fallback", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      null,
      previousStockAvatarUrl,
      currentStockAvatarUrl,
      stockAvatarUrls,
    ),
    currentStockAvatarUrl,
  );
});

test("running agent card preserves a custom definition fallback", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      " ",
      " https://relay.example/custom-definition.png ",
      currentStockAvatarUrl,
      stockAvatarUrls,
    ),
    "https://relay.example/custom-definition.png",
  );
});

test("missing catalog metadata preserves the existing fallback order", () => {
  assert.equal(
    resolveAgentCardAvatarUrl(
      previousStockAvatarUrl,
      "https://relay.example/definition.png",
      currentStockAvatarUrl,
    ),
    previousStockAvatarUrl,
  );
  assert.equal(
    resolveAgentCardAvatarUrl(
      null,
      "https://relay.example/definition.png",
      currentStockAvatarUrl,
    ),
    "https://relay.example/definition.png",
  );
});

test("running agent card trims and ignores blank avatar values", () => {
  assert.equal(resolveAgentCardAvatarUrl("  ", ""), null);
  assert.equal(
    resolveAgentCardAvatarUrl(" ", " ", " current.png ", stockAvatarUrls),
    "current.png",
  );
  assert.equal(resolveAgentCardAvatarUrl(" ", " ", " ", stockAvatarUrls), null);
});

test("linked agent actions wait for the authoritative profile avatar", () => {
  assert.equal(isAgentCardAvatarLoading(true, true), true);
  assert.equal(isAgentCardAvatarLoading(true, false), false);
});

test("unlinked persona actions do not wait for a profile", () => {
  assert.equal(isAgentCardAvatarLoading(false, true), false);
});
