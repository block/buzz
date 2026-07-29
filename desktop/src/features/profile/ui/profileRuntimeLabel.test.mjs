/**
 * How the profile surfaces name a harness.
 *
 * The table these helpers wrap knows four command strings, none of which is a
 * command the fleet actually pins: a `hermes --profile marshall acp` record
 * fell through to the bare word "hermes", and a second profile of the same
 * harness was indistinguishable from the first. A provider-backed record must
 * answer from its pin; a local one must not move.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  managedAgentRuntimeCopyValue,
  managedAgentRuntimeLabel,
  runtimeCommandLabel,
} from "./profileRuntimeLabel.ts";

const localAgent = {
  backend: { type: "local" },
  agentCommand: "goose",
  agentArgs: [],
};

const remoteAgent = {
  backend: { type: "provider", id: "ssh", config: { ssh_host: "vps" } },
  agentCommand: "hermes",
  agentArgs: ["--profile", "marshall", "acp"],
};

test("a relay agent's declared type keeps its friendly name", () => {
  assert.equal(runtimeCommandLabel("codex-acp"), "Codex");
  assert.equal(runtimeCommandLabel("claude-code"), "Claude Code");
  assert.equal(runtimeCommandLabel("goose"), "Goose");
  // A harness Buzz cannot run itself, so it lives in HARNESS_LABELS only for
  // this surface — see `NOT_IN_RUST_CATALOG` in `pinnedHarness.test.mjs`.
  assert.equal(runtimeCommandLabel("aider"), "Aider");
});

test("every harness the catalog knows is named here too", () => {
  // The four-entry table this used to carry named none of these, so a relay
  // agent declaring one read as a raw command beside a record that read as a
  // name. One owner means a harness learned in Rust arrives on both surfaces.
  assert.equal(runtimeCommandLabel("hermes"), "Hermes Agent");
  assert.equal(runtimeCommandLabel("opencode"), "OpenCode");
  assert.equal(runtimeCommandLabel("amp-acp"), "Amp");
});

test("an unmapped command names itself", () => {
  assert.equal(runtimeCommandLabel("acme-brain"), "acme-brain");
});

test("a local record resolves exactly as it did", () => {
  assert.equal(managedAgentRuntimeLabel(localAgent), "Goose");
  assert.equal(managedAgentRuntimeCopyValue(localAgent), "goose");
});

test("a remote record is named by its pin, profile and all", () => {
  assert.equal(
    managedAgentRuntimeLabel(remoteAgent),
    "Hermes Agent (marshall)",
  );
});

test("two profiles of one harness are two names", () => {
  assert.notEqual(
    managedAgentRuntimeLabel(remoteAgent),
    managedAgentRuntimeLabel({
      ...remoteAgent,
      agentArgs: ["--profile", "matt", "acp"],
    }),
  );
});

test("copying a remote harness yields the command that runs there", () => {
  // The bare `agentCommand` would paste a `hermes` that starts the wrong
  // profile on the host.
  assert.equal(
    managedAgentRuntimeCopyValue(remoteAgent),
    "hermes --profile marshall acp",
  );
});

test("an unknown remote binary shows itself rather than a local guess", () => {
  const agent = {
    ...remoteAgent,
    agentCommand: "/opt/acme/acme-brain",
    agentArgs: ["serve"],
  };
  assert.equal(managedAgentRuntimeLabel(agent), "/opt/acme/acme-brain");
});
