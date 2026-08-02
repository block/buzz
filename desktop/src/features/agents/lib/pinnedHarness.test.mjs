/**
 * Harness-pin identity.
 *
 * A provider-backed record's `agentCommand`/`agentArgs` name a binary on the
 * HOST, which this computer's runtime catalog has never seen. These cases pin
 * the two real fleet shapes that broke — a `hermes --profile <name> acp` pin
 * that rendered a generic icon, and a `claude-agent-acp` pin that only looked
 * right by name collision with a local builtin — plus the label table's
 * agreement with the Rust catalogs it mirrors.
 */

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { PRESET_LOGOS } from "../../onboarding/ui/RuntimeIcon.tsx";
import {
  HARNESS_LABELS,
  providerRecordHarness,
  resolvePinnedHarness,
} from "./pinnedHarness.ts";

// ── The label table mirrors the Rust catalogs ────────────────────────────────
//
// `HARNESS_LABELS` restates ids and labels that live in Rust. The two sides are
// different languages, so no compiler catches drift and a renamed harness would
// silently fall back to rendering its raw command. Same trick as
// `presetLogos.test.mjs`: read the Rust source as text.

const desktopRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../../..",
);

const discoveryRs = readFileSync(
  path.join(desktopRoot, "src-tauri/src/managed_agents/discovery.rs"),
  "utf8",
);

/** Every `id` + `label` pair inside one Rust table literal. */
function parseCatalog(constName, structName) {
  const block = discoveryRs.match(
    new RegExp(
      `const ${constName}: &\\[${structName}\\] = &\\[([\\s\\S]*?)\\n\\];`,
    ),
  );
  assert.ok(block, `could not locate ${constName} in discovery.rs`);
  return [
    ...block[1].matchAll(/^\s{8}id: "([^"]+)",\n\s{8}label: "([^"]+)",$/gm),
  ].map((match) => ({ id: match[1], label: match[2] }));
}

const rustHarnesses = [
  ...parseCatalog("KNOWN_ACP_RUNTIMES", "KnownAcpRuntime"),
  ...parseCatalog("PRESET_HARNESSES", "PresetHarness"),
];

test("the Rust catalog parse found both tables", () => {
  // Guards the regex itself: a struct-field reorder would otherwise yield zero
  // pairs and make every assertion below vacuously pass.
  assert.ok(
    rustHarnesses.length >= 12,
    `expected at least 12 harnesses, parsed ${rustHarnesses.length}`,
  );
});

/**
 * Keys that intentionally have no Rust counterpart.
 *
 * The table also names the free-form command strings foreign surfaces carry —
 * a relay agent's self-declared `agentType` — which are harnesses Buzz itself
 * cannot run and so appear in no local catalog. Each one is listed with its
 * reason, so an id dropped from the Rust side cannot hide here.
 */
const NOT_IN_RUST_CATALOG = new Set([
  // Declared by relay agents (`agentType`); Buzz has no Aider runtime entry.
  "aider",
]);

for (const { id, label } of rustHarnesses) {
  test(`harness "${id}" renders its catalog label`, () => {
    // Resolved through the public helper rather than the private table, so a
    // pin whose command IS the id keeps proving the whole path.
    assert.equal(
      resolvePinnedHarness(id, []).label,
      label,
      `"${id}" falls back to its raw command instead of "${label}" — add it ` +
        "to HARNESS_LABELS in pinnedHarness.ts.",
    );
  });
}

test("HARNESS_LABELS names no harness the Rust catalogs dropped", () => {
  // The other direction. Without it a harness renamed or removed in Rust
  // leaves a stale TS entry that keeps answering with the old name, and the
  // per-id tests above — which only walk the Rust side — stay green.
  const rustIds = new Set(rustHarnesses.map((harness) => harness.id));
  const orphaned = Object.keys(HARNESS_LABELS).filter(
    (id) => !rustIds.has(id) && !NOT_IN_RUST_CATALOG.has(id),
  );
  assert.deepEqual(
    orphaned,
    [],
    `HARNESS_LABELS names ids no Rust catalog emits: ${orphaned.join(", ")}. ` +
      "Drop them, or list them in NOT_IN_RUST_CATALOG with the surface that " +
      "carries the command.",
  );
});

// ── The pins that broke ─────────────────────────────────────────────────────

test("a hermes profile pin is named and marked", () => {
  const pin = resolvePinnedHarness("hermes", ["--profile", "marshall", "acp"]);
  assert.equal(pin.id, "hermes");
  assert.equal(
    pin.label,
    "Hermes Agent (marshall)",
    "the profile is the identity — two profiles of one harness are two agents",
  );
  assert.equal(pin.logoUrl, PRESET_LOGOS.hermes);
  assert.equal(pin.command, "hermes --profile marshall acp");
});

test("a claude pin resolves through its adapter command", () => {
  const pin = resolvePinnedHarness("claude-agent-acp", []);
  assert.equal(pin.id, "claude");
  assert.equal(pin.label, "Claude Code");
  assert.ok(pin.logoUrl, "the bundled claude mark, not the generic icon");
  assert.equal(pin.command, "claude-agent-acp");
});

test("an unknown host binary shows itself rather than a guess", () => {
  const pin = resolvePinnedHarness("/opt/acme/acme-brain", ["serve", "--acp"]);
  assert.equal(pin.id, null, "no id was earned, so none is claimed");
  assert.equal(
    pin.label,
    "/opt/acme/acme-brain",
    "the pin itself is the honest label — never a local default",
  );
  assert.equal(pin.logoUrl, null);
  assert.equal(pin.command, "/opt/acme/acme-brain serve --acp");
});

test("a host path and extension do not hide a known harness", () => {
  assert.equal(
    resolvePinnedHarness("/home/ubuntu/.local/bin/hermes-acp", []).label,
    "Hermes Agent",
    "the pin describes the HOST's filesystem, which is not this computer's",
  );
  assert.equal(
    resolvePinnedHarness("C:\\tools\\Codex-ACP.EXE", []).label,
    "Codex",
  );
});

test("an unmapped base is not shortened into an identity it did not earn", () => {
  // `buzz-agent` is a known id whole; `acme-agent` must not become "acme".
  assert.equal(resolvePinnedHarness("buzz-agent", []).label, "Buzz Agent");
  assert.equal(resolvePinnedHarness("acme-agent", []).id, null);
  assert.equal(resolvePinnedHarness("-hermes", []).id, null);
});

test("an empty pin says so", () => {
  const pin = resolvePinnedHarness("   ", []);
  assert.equal(pin.id, null);
  assert.equal(pin.label, "Not configured");
  assert.equal(pin.command, "");
});

// ── Profile parsing ─────────────────────────────────────────────────────────

test("both spellings of the profile flag are read", () => {
  assert.equal(
    resolvePinnedHarness("hermes", ["--profile=matt", "acp"]).label,
    "Hermes Agent (matt)",
  );
});

test("a profile flag without a value names no profile", () => {
  // `--profile --verbose` means the flag was passed bare; naming the agent
  // "Hermes Agent (--verbose)" would be worse than not narrowing at all.
  assert.equal(
    resolvePinnedHarness("hermes", ["--profile", "--verbose"]).label,
    "Hermes Agent",
  );
  assert.equal(
    resolvePinnedHarness("hermes", ["--profile"]).label,
    "Hermes Agent",
  );
  assert.equal(
    resolvePinnedHarness("hermes", ["--profile="]).label,
    "Hermes Agent",
  );
});

test("an unknown command still takes its profile", () => {
  assert.equal(
    resolvePinnedHarness("acme-brain", ["--profile", "ops"]).label,
    "acme-brain (ops)",
  );
});

// ── Local records are untouched ─────────────────────────────────────────────

const localAgent = {
  backend: { type: "local" },
  agentCommand: "claude-agent-acp",
  agentArgs: [],
};

const remoteAgent = {
  backend: { type: "provider", id: "ssh", config: { ssh_host: "vps" } },
  agentCommand: "hermes",
  agentArgs: ["--profile", "marshall", "acp"],
};

test("a local record has no pin to read", () => {
  // A local agent runs on this computer, where the catalog genuinely describes
  // it; every local surface must keep resolving exactly as it did.
  assert.equal(providerRecordHarness(localAgent), null);
});

test("a provider record answers from itself", () => {
  const pin = providerRecordHarness(remoteAgent);
  assert.equal(pin?.label, "Hermes Agent (marshall)");
  assert.equal(pin?.logoUrl, PRESET_LOGOS.hermes);
});
