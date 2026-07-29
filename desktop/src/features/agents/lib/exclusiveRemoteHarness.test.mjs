/**
 * The exclusive-identity guard: "is this catalog entry already driven by an
 * agent I have?"
 *
 * An exclusive entry names a persistent identity on the host (its own memory,
 * sessions, credentials), so a second agent pinned to it would be a second
 * puppeteer on one body. A non-exclusive entry is an ephemeral runner and may
 * be deployed as many times as the user likes — these tests pin both halves.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  addedExclusiveHarnessIds,
  isExclusiveRemoteHarnessAdded,
} from "./exclusiveRemoteHarness.ts";
import { resolvePinnedHarness } from "./pinnedHarness.ts";

const HOST = {
  providerId: "ssh",
  config: { ssh_host: "vps", ssh_user: "bee" },
};

function harness(overrides = {}) {
  return {
    id: "hermes-default",
    label: "Hermes (default)",
    command: "hermes",
    args: ["--profile", "default", "acp"],
    env: {},
    available: true,
    binaryPath: "/usr/local/bin/hermes",
    version: null,
    exclusive: true,
    ...overrides,
  };
}

function agent(overrides = {}) {
  return {
    pubkey: "a".repeat(64),
    name: "Hermes",
    agentCommand: "hermes",
    agentCommandOverride: null,
    agentArgs: ["--profile", "default", "acp"],
    backend: { type: "provider", id: "ssh", config: { ...HOST.config } },
    ...overrides,
  };
}

test("same host and same pinned identity reads as added", () => {
  assert.equal(isExclusiveRemoteHarnessAdded(harness(), HOST, [agent()]), true);
});

test("no agents at all means nothing is added", () => {
  assert.equal(isExclusiveRemoteHarnessAdded(harness(), HOST, []), false);
});

test("the same identity on a different host is a different identity", () => {
  const elsewhere = agent({
    backend: {
      type: "provider",
      id: "ssh",
      config: { ssh_host: "other-vps", ssh_user: "bee" },
    },
  });
  assert.equal(
    isExclusiveRemoteHarnessAdded(harness(), HOST, [elsewhere]),
    false,
  );
});

test("the same host reached as a different user is a different identity store", () => {
  // A different $HOME is different memory/sessions, so it must not block.
  const otherUser = agent({
    backend: {
      type: "provider",
      id: "ssh",
      config: { ssh_host: "vps", ssh_user: "root" },
    },
  });
  assert.equal(
    isExclusiveRemoteHarnessAdded(harness(), HOST, [otherUser]),
    false,
  );
});

test("another provider on the same-looking config does not match", () => {
  const otherProvider = agent({
    backend: { type: "provider", id: "fly", config: { ...HOST.config } },
  });
  assert.equal(
    isExclusiveRemoteHarnessAdded(harness(), HOST, [otherProvider]),
    false,
  );
});

test("a different profile on the same host does not match", () => {
  const matt = agent({ agentArgs: ["--profile", "matt", "acp"] });
  assert.equal(isExclusiveRemoteHarnessAdded(harness(), HOST, [matt]), false);
});

test("a different command with identical args does not match", () => {
  const other = agent({ agentCommand: "hermes-next" });
  assert.equal(isExclusiveRemoteHarnessAdded(harness(), HOST, [other]), false);
});

test("a local agent can never occupy a host identity", () => {
  const local = agent({ backend: { type: "local" } });
  assert.equal(isExclusiveRemoteHarnessAdded(harness(), HOST, [local]), false);
});

test("a non-exclusive entry is never added, however many agents run it", () => {
  const claude = harness({
    id: "claude",
    label: "Claude Code",
    command: "claude-code-acp",
    args: [],
    exclusive: undefined,
  });
  const running = agent({ agentCommand: "claude-code-acp", agentArgs: [] });
  assert.equal(
    isExclusiveRemoteHarnessAdded(claude, HOST, [running, running]),
    false,
    "ephemeral runners are meant to be deployed N times",
  );
});

test("an absent exclusive flag behaves exactly like today (no guard)", () => {
  const legacy = harness();
  delete legacy.exclusive;
  assert.equal(isExclusiveRemoteHarnessAdded(legacy, HOST, [agent()]), false);
});

test("blank and untrimmed args still match the record they minted", () => {
  // `create_time_agent_args` trims and drops blanks on the way into the
  // record, so the catalog entry must be compared the same way.
  const messy = harness({ args: [" --profile ", "default", "", "acp"] });
  assert.equal(isExclusiveRemoteHarnessAdded(messy, HOST, [agent()]), true);
});

test("what counts as the same pin here is what the user is shown", () => {
  // Two spellings of one rule is the drift this shares an owner to avoid: a
  // pin taken by the guard must read as the same string on screen, or an
  // agent is "already added" against a card naming something else.
  const messy = { command: " hermes ", args: [" --profile ", "default", ""] };
  const clean = { command: "hermes", args: ["--profile", "default"] };
  assert.equal(
    resolvePinnedHarness(messy.command, messy.args).command,
    resolvePinnedHarness(clean.command, clean.args).command,
  );
  assert.equal(
    isExclusiveRemoteHarnessAdded(harness({ args: messy.args }), HOST, [
      agent({ agentArgs: clean.args }),
    ]),
    true,
  );
});

test("a blank optional config field equals an omitted one", () => {
  // The create dialog seeds schema defaults, so both spellings really occur.
  const seeded = {
    providerId: "ssh",
    config: { ssh_host: "vps", ssh_user: "bee", ssh_port: "" },
  };
  assert.equal(
    isExclusiveRemoteHarnessAdded(harness(), seeded, [agent()]),
    true,
  );
});

test("config key order does not change the answer", () => {
  const reordered = agent({
    backend: {
      type: "provider",
      id: "ssh",
      config: { ssh_user: "bee", ssh_host: "vps" },
    },
  });
  assert.equal(
    isExclusiveRemoteHarnessAdded(harness(), HOST, [reordered]),
    true,
  );
});

test("addedExclusiveHarnessIds collects only the taken exclusive entries", () => {
  const catalog = [
    // Taken.
    harness(),
    // Exclusive but free: nobody drives this profile.
    harness({ id: "hermes-matt", args: ["--profile", "matt", "acp"] }),
    // Running, but an ephemeral runner: never "taken".
    harness({
      id: "claude",
      command: "claude-code-acp",
      args: [],
      exclusive: undefined,
    }),
    harness({
      id: "codex",
      command: "codex",
      args: ["acp"],
      exclusive: false,
    }),
  ];
  const ids = addedExclusiveHarnessIds(catalog, HOST, [
    agent(),
    agent({ agentCommand: "claude-code-acp", agentArgs: [] }),
    agent({ agentCommand: "codex", agentArgs: ["acp"] }),
  ]);
  assert.deepEqual([...ids], ["hermes-default"]);
});

test("addedExclusiveHarnessIds is empty for an empty catalog", () => {
  assert.equal(addedExclusiveHarnessIds([], HOST, [agent()]).size, 0);
});
