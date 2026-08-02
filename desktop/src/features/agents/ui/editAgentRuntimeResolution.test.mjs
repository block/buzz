/**
 * Which runtime the Edit Agent dialog is talking about.
 *
 * The dangerous half is the provider-backed record: its harness runs on the
 * HOST, so every answer this computer's catalog gives is a miss or a name
 * collision, and that answer goes on to decide the dialog's harness label, its
 * credential questions, and whether Save is allowed to write `provider: null`.
 * These cases pin the two fleet records that broke — a `hermes` pin the local
 * catalog cannot see at all, and a `claude-agent-acp` pin that collides with a
 * local builtin's command — alongside the local paths, which must not move.
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  resolveDialogRuntimeId,
  resolveOriginalRuntimeSupportsProvider,
  resolveProspectiveRuntimeId,
} from "./editAgentRuntimeResolution.ts";

/** The local catalog, as this computer reports it. */
const runtimes = [
  {
    id: "buzz-agent",
    command: "/usr/local/bin/buzz-agent",
    defaultArgs: [],
    availability: "available",
  },
  {
    id: "claude",
    command: "claude-agent-acp",
    defaultArgs: [],
    availability: "available",
  },
  {
    id: "codex",
    command: "codex-acp",
    defaultArgs: [],
    availability: "available",
  },
];

// ── resolveDialogRuntimeId: which catalog row the dropdown preselects ────────

test("a local record matches the catalog by command", () => {
  assert.equal(
    resolveDialogRuntimeId(runtimes, "claude-agent-acp", false),
    "claude",
  );
});

test("a local record also matches by id", () => {
  assert.equal(
    resolveDialogRuntimeId(runtimes, "buzz-agent", false),
    "buzz-agent",
  );
});

test("a local record with an unknown command matches nothing", () => {
  assert.equal(
    resolveDialogRuntimeId(runtimes, "/opt/acme/brain", false),
    null,
  );
});

test("a provider record refuses the collision it used to accept", () => {
  // `claude-agent-acp` is a local builtin's command, which is the only reason a
  // REMOTE Claude agent's dropdown ever looked correct. Matching it would then
  // point local model discovery at this computer's Claude and present its
  // models as the host's.
  assert.equal(
    resolveDialogRuntimeId(runtimes, "claude-agent-acp", true),
    null,
    "a name collision is not knowledge of the host",
  );
});

test("a provider record with a harness this computer lacks also answers null", () => {
  assert.equal(resolveDialogRuntimeId(runtimes, "hermes", true), null);
});

// ── resolveOriginalRuntimeSupportsProvider ──────────────────────────────────

test("the opening runtime's provider capability comes from its command", () => {
  assert.equal(
    resolveOriginalRuntimeSupportsProvider(
      runtimes,
      "/usr/local/bin/buzz-agent",
    ),
    true,
  );
  assert.equal(
    resolveOriginalRuntimeSupportsProvider(runtimes, "claude-agent-acp"),
    false,
  );
});

test("an unmatched command claims no provider capability", () => {
  assert.equal(
    resolveOriginalRuntimeSupportsProvider(runtimes, "hermes"),
    false,
  );
});

// ── resolveProspectiveRuntimeId: what will be active after Save ─────────────

const localInherit = {
  runtimes,
  pinnedRuntimeId: null,
  inheritHarness: true,
  personaRuntimeId: null,
  agentCommand: "claude-agent-acp",
  selectedRuntimeId: "claude",
};

test("a pinned remote harness answers from itself and stops", () => {
  assert.equal(
    resolveProspectiveRuntimeId({
      ...localInherit,
      pinnedRuntimeId: "hermes",
      // Everything below would otherwise win; none of it describes the host.
      personaRuntimeId: "buzz-agent",
      selectedRuntimeId: "buzz-agent",
    }),
    "hermes",
  );
});

test("an unrecognized remote pin is empty, never the local default", () => {
  // The bug in one line: with no pin to speak for it, the fallback chain below
  // lands on this computer's default runtime — the "Buzz Agent" a Hermes agent
  // was labeled with, and the id its credential questions were asked of. `""`
  // is the same honest answer `createGateHarnessId` gives for an unknown
  // remote harness.
  assert.equal(
    resolveProspectiveRuntimeId({
      ...localInherit,
      pinnedRuntimeId: "",
      personaRuntimeId: null,
      agentCommand: "/opt/acme/acme-brain",
    }),
    "",
  );
});

test("a pinned local selection is the selected runtime", () => {
  assert.equal(
    resolveProspectiveRuntimeId({
      ...localInherit,
      inheritHarness: false,
      selectedRuntimeId: "codex",
    }),
    "codex",
  );
});

test("a pinned local selection outside the catalog passes through", () => {
  assert.equal(
    resolveProspectiveRuntimeId({
      ...localInherit,
      inheritHarness: false,
      selectedRuntimeId: "custom",
    }),
    "custom",
  );
});

test("inheriting resolves the template's runtime, not the still-present pin", () => {
  // The record still carries its Claude override at this moment; what will run
  // once the override clears is the template's runtime.
  assert.equal(
    resolveProspectiveRuntimeId({
      ...localInherit,
      personaRuntimeId: "buzz-agent",
    }),
    "buzz-agent",
  );
});

test("inheriting a template runtime this computer lacks keeps the id", () => {
  assert.equal(
    resolveProspectiveRuntimeId({ ...localInherit, personaRuntimeId: "goose" }),
    "goose",
  );
});

test("inheriting with no template runtime falls back to the record's command", () => {
  assert.equal(resolveProspectiveRuntimeId(localInherit), "claude");
});

test("inheriting with nothing to go on falls back to the app default", () => {
  assert.equal(
    resolveProspectiveRuntimeId({
      ...localInherit,
      agentCommand: "/opt/acme/acme-brain",
    }),
    "buzz-agent",
    "a definition with no runtime still gets a runtime discovery can run",
  );
});
