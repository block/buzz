import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  sortedPresetEntries,
  customEntries,
  isEditableEntry,
  countAgentsReferencingHarness,
  deleteHarnessConfirmMessage,
  deleteConfirmState,
} from "./harnessGalleryLogic.ts";

// ── Minimal catalog entry factory ────────────────────────────────────────────

function entry(overrides = {}) {
  return {
    id: overrides.id ?? "test-id",
    label: overrides.label ?? "Test",
    source: overrides.source ?? "custom",
    availability: overrides.availability ?? "not_installed",
    avatarUrl: "",
    command: overrides.command ?? null,
    binaryPath: null,
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
  };
}

// ── sortedPresetEntries ───────────────────────────────────────────────────────

describe("sortedPresetEntries", () => {
  it("returns only preset-source entries", () => {
    const catalog = [
      entry({ id: "p1", source: "preset" }),
      entry({ id: "c1", source: "custom" }),
      entry({ id: "b1", source: "builtin" }),
    ];
    const result = sortedPresetEntries(catalog);
    assert.equal(result.length, 1);
    assert.equal(result[0].id, "p1");
  });

  it("places detected (available) entries before not-installed", () => {
    const catalog = [
      entry({
        id: "not-there",
        source: "preset",
        availability: "not_installed",
        label: "Alpha",
      }),
      entry({
        id: "detected",
        source: "preset",
        availability: "available",
        label: "Beta",
      }),
    ];
    const result = sortedPresetEntries(catalog);
    assert.equal(result[0].id, "detected", "detected entry must come first");
    assert.equal(result[1].id, "not-there");
  });

  it("sorts alphabetically within detected group", () => {
    const catalog = [
      entry({
        id: "z",
        source: "preset",
        availability: "available",
        label: "Zebra",
      }),
      entry({
        id: "a",
        source: "preset",
        availability: "available",
        label: "Aardvark",
      }),
    ];
    const result = sortedPresetEntries(catalog);
    assert.equal(result[0].id, "a");
    assert.equal(result[1].id, "z");
  });

  it("sorts alphabetically within not-installed group", () => {
    const catalog = [
      entry({
        id: "z",
        source: "preset",
        availability: "not_installed",
        label: "Zebra",
      }),
      entry({
        id: "a",
        source: "preset",
        availability: "not_installed",
        label: "Aardvark",
      }),
    ];
    const result = sortedPresetEntries(catalog);
    assert.equal(result[0].id, "a");
    assert.equal(result[1].id, "z");
  });

  it("returns empty array when no preset entries", () => {
    const catalog = [entry({ source: "custom" }), entry({ source: "builtin" })];
    assert.deepEqual(sortedPresetEntries(catalog), []);
  });

  it("does not mutate the input array", () => {
    const catalog = [
      entry({
        id: "z",
        source: "preset",
        availability: "available",
        label: "Z",
      }),
      entry({
        id: "a",
        source: "preset",
        availability: "available",
        label: "A",
      }),
    ];
    const original = [...catalog];
    sortedPresetEntries(catalog);
    assert.deepEqual(
      catalog.map((e) => e.id),
      original.map((e) => e.id),
      "input array must not be mutated",
    );
  });
});

// ── customEntries ─────────────────────────────────────────────────────────────

describe("customEntries", () => {
  it("returns only custom-source entries", () => {
    const catalog = [
      entry({ id: "p1", source: "preset" }),
      entry({ id: "c1", source: "custom" }),
      entry({ id: "c2", source: "custom" }),
    ];
    const result = customEntries(catalog);
    assert.equal(result.length, 2);
    assert.ok(result.every((e) => e.source === "custom"));
  });

  it("returns empty when no custom entries", () => {
    const catalog = [entry({ source: "preset" }), entry({ source: "builtin" })];
    assert.deepEqual(customEntries(catalog), []);
  });
});

// ── isEditableEntry ───────────────────────────────────────────────────────────

describe("isEditableEntry", () => {
  it("returns true for custom entries", () => {
    assert.ok(isEditableEntry(entry({ source: "custom" })));
  });

  it("returns false for preset entries", () => {
    assert.ok(!isEditableEntry(entry({ source: "preset" })));
  });

  it("returns false for builtin entries", () => {
    assert.ok(!isEditableEntry(entry({ source: "builtin" })));
  });
});

// ── countAgentsReferencingHarness ─────────────────────────────────────────────

describe("countAgentsReferencingHarness", () => {
  it("counts direct record-level runtime pins", () => {
    const agents = [
      { runtime: "my-harness", personaId: null },
      { runtime: "other", personaId: null },
      { runtime: "my-harness", personaId: "p1" },
    ];
    assert.equal(countAgentsReferencingHarness("my-harness", agents, []), 2);
  });

  it("counts persona-inherited references when record runtime is null", () => {
    const agents = [{ runtime: null, personaId: "p1" }];
    const personas = [{ id: "p1", runtime: "my-harness" }];
    assert.equal(
      countAgentsReferencingHarness("my-harness", agents, personas),
      1,
    );
  });

  it("record-level pin shadows persona runtime (no double counting, pin wins)", () => {
    // Agent pinned to "other" whose persona uses "my-harness" does NOT count:
    // the effective harness is the pin.
    const agents = [{ runtime: "other", personaId: "p1" }];
    const personas = [{ id: "p1", runtime: "my-harness" }];
    assert.equal(
      countAgentsReferencingHarness("my-harness", agents, personas),
      0,
    );
  });

  it("agents with no runtime and no persona do not count", () => {
    const agents = [{ runtime: null, personaId: null }];
    assert.equal(countAgentsReferencingHarness("my-harness", agents, []), 0);
  });

  it("persona reference to a different harness does not count", () => {
    const agents = [{ runtime: null, personaId: "p1" }];
    const personas = [{ id: "p1", runtime: "other" }];
    assert.equal(
      countAgentsReferencingHarness("my-harness", agents, personas),
      0,
    );
  });
});

// ── deleteHarnessConfirmMessage ───────────────────────────────────────────────

describe("deleteHarnessConfirmMessage", () => {
  it("plain confirmation when nothing references the harness", () => {
    assert.equal(
      deleteHarnessConfirmMessage("My Harness", 0),
      "Delete My Harness?",
    );
  });

  it("singular copy for one referencing agent", () => {
    assert.equal(
      deleteHarnessConfirmMessage("My Harness", 1),
      "1 agent uses this harness and will stop launching. Delete My Harness?",
    );
  });

  it("plural copy for multiple referencing agents", () => {
    assert.equal(
      deleteHarnessConfirmMessage("My Harness", 3),
      "3 agents use this harness and will stop launching. Delete My Harness?",
    );
  });
});

// ── deleteConfirmState ────────────────────────────────────────────────────────

describe("deleteConfirmState", () => {
  const settled = (data) => ({ isPending: false, isError: false, data });
  const pending = { isPending: true, isError: false, data: undefined };
  const failed = { isPending: false, isError: true, data: undefined };

  it("disables confirm while agents query is still loading", () => {
    const state = deleteConfirmState("h1", "My Harness", pending, settled([]));
    assert.equal(state.canConfirm, false);
    assert.match(state.message, /Checking which agents/);
  });

  it("disables confirm while personas query is still loading", () => {
    const state = deleteConfirmState("h1", "My Harness", settled([]), pending);
    assert.equal(state.canConfirm, false);
    assert.match(state.message, /Checking which agents/);
  });

  it("query failure does not claim zero dependents", () => {
    const state = deleteConfirmState("h1", "My Harness", failed, settled([]));
    assert.equal(state.canConfirm, true);
    assert.match(state.message, /Couldn't check/);
    assert.doesNotMatch(state.message, /^Delete My Harness\?$/);
  });

  it("persona query failure also reports unknown blast radius", () => {
    const state = deleteConfirmState("h1", "My Harness", settled([]), failed);
    assert.equal(state.canConfirm, true);
    assert.match(state.message, /Couldn't check/);
  });

  it("settled queries produce the counted warning and enable confirm", () => {
    const agents = settled([
      { runtime: "h1", personaId: null },
      { runtime: null, personaId: "p1" },
    ]);
    const personas = settled([{ id: "p1", runtime: "h1" }]);
    const state = deleteConfirmState("h1", "My Harness", agents, personas);
    assert.equal(state.canConfirm, true);
    assert.equal(
      state.message,
      "2 agents use this harness and will stop launching. Delete My Harness?",
    );
  });

  it("settled queries with zero dependents use the plain confirmation", () => {
    const state = deleteConfirmState(
      "h1",
      "My Harness",
      settled([]),
      settled([]),
    );
    assert.equal(state.canConfirm, true);
    assert.equal(state.message, "Delete My Harness?");
  });
});
