import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  catalogDialogEntries,
  catalogPrimaryAction,
  entryStatusLabel,
  filterCatalogEntries,
  isYourHarnessEntry,
  stableRowOrder,
  yourHarnessEntries,
} from "./harnessCatalogLogic.ts";

// ── Minimal catalog entry factory ────────────────────────────────────────────

function entry(overrides = {}) {
  return {
    id: "test-id",
    label: "Test",
    source: "preset",
    availability: "not_installed",
    avatarUrl: "",
    command: null,
    binaryPath: null,
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    requiresExternalCli: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
    ...overrides,
  };
}

// ── isYourHarnessEntry / yourHarnessEntries ──────────────────────────────────

describe("isYourHarnessEntry", () => {
  it("includes available entries", () => {
    assert.equal(
      isYourHarnessEntry(entry({ availability: "available" })),
      true,
    );
  });

  it("includes one-click installable entries (auto-install, no node gate)", () => {
    assert.equal(
      isYourHarnessEntry(entry({ canAutoInstall: true, nodeRequired: false })),
      true,
    );
  });

  it("excludes auto-installable entries blocked on Node.js", () => {
    assert.equal(
      isYourHarnessEntry(entry({ canAutoInstall: true, nodeRequired: true })),
      false,
    );
  });

  it("excludes presets needing manual setup", () => {
    assert.equal(
      isYourHarnessEntry(
        entry({ availability: "cli_missing", canAutoInstall: false }),
      ),
      false,
    );
  });

  it("always includes custom entries regardless of readiness", () => {
    assert.equal(
      isYourHarnessEntry(
        entry({ source: "custom", availability: "not_installed" }),
      ),
      true,
    );
  });
});

describe("yourHarnessEntries", () => {
  it("filters to owned/ready rows only", () => {
    const catalog = [
      entry({ id: "ready", availability: "available" }),
      entry({ id: "needs-setup", availability: "cli_missing" }),
      entry({ id: "mine", source: "custom" }),
    ];
    assert.deepEqual(
      yourHarnessEntries(catalog).map((e) => e.id),
      ["ready", "mine"],
    );
  });
});

// ── catalogDialogEntries ─────────────────────────────────────────────────────

describe("catalogDialogEntries", () => {
  it("excludes custom entries and sorts needs-setup first, alpha within group", () => {
    const catalog = [
      entry({ id: "zed", label: "Zed", availability: "available" }),
      entry({ id: "mine", label: "Mine", source: "custom" }),
      entry({ id: "beta", label: "Beta", availability: "cli_missing" }),
      entry({ id: "alpha", label: "Alpha", availability: "not_installed" }),
    ];
    assert.deepEqual(
      catalogDialogEntries(catalog).map((e) => e.id),
      ["alpha", "beta", "zed"],
    );
  });
});

// ── filterCatalogEntries ─────────────────────────────────────────────────────

describe("filterCatalogEntries", () => {
  const entries = [
    entry({ id: "kimi", label: "Kimi Code", command: "kimi" }),
    entry({ id: "amp", label: "Amp", command: "amp" }),
    entry({ id: "omp", label: "Oh My Pi", command: "omp" }),
  ];

  it("returns everything for a blank query", () => {
    assert.equal(filterCatalogEntries(entries, "  ").length, 3);
  });

  it("matches label case-insensitively", () => {
    assert.deepEqual(
      filterCatalogEntries(entries, "kIMi").map((e) => e.id),
      ["kimi"],
    );
  });

  it("matches command", () => {
    assert.deepEqual(
      filterCatalogEntries(entries, "omp").map((e) => e.id),
      ["omp"],
    );
  });

  it("returns empty for no match", () => {
    assert.equal(filterCatalogEntries(entries, "zzz").length, 0);
  });
});

// ── stableRowOrder ───────────────────────────────────────────────────────────

describe("stableRowOrder", () => {
  it("initial order: priority builtins, then enabled, then alpha", () => {
    const entries = [
      entry({ id: "zeta", label: "Zeta", availability: "available" }),
      entry({
        id: "goose",
        label: "goose",
        availability: "available",
        source: "builtin",
      }),
      entry({ id: "off", label: "Aardvark", canAutoInstall: true }),
      entry({
        id: "buzz-agent",
        label: "Buzz",
        availability: "available",
        source: "builtin",
      }),
    ];
    assert.deepEqual(stableRowOrder([], entries), [
      "buzz-agent",
      "goose",
      "zeta",
      "off",
    ]);
  });

  it("keeps previous relative order when availability flips (no reorder on toggle)", () => {
    const before = ["buzz-agent", "zeta", "off"];
    const entries = [
      entry({ id: "off", label: "Aardvark", availability: "available" }), // just installed
      entry({ id: "zeta", label: "Zeta", availability: "available" }),
      entry({
        id: "buzz-agent",
        label: "Buzz",
        availability: "available",
        source: "builtin",
      }),
    ];
    assert.deepEqual(stableRowOrder(before, entries), [
      "buzz-agent",
      "zeta",
      "off",
    ]);
  });

  it("drops removed ids and appends newcomers at the end", () => {
    const before = ["buzz-agent", "deleted", "zeta"];
    const entries = [
      entry({ id: "zeta", label: "Zeta", availability: "available" }),
      entry({ id: "buzz-agent", label: "Buzz", availability: "available" }),
      entry({ id: "new-one", label: "New One", availability: "available" }),
    ];
    assert.deepEqual(stableRowOrder(before, entries), [
      "buzz-agent",
      "zeta",
      "new-one",
    ]);
  });
});

// ── entryStatusLabel ─────────────────────────────────────────────────────────

describe("entryStatusLabel", () => {
  it("config error wins over availability", () => {
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          authStatus: { status: "config_invalid", diagnostic: "boom" },
        }),
      ),
      "Config error",
    );
  });

  it("maps availability states", () => {
    assert.equal(
      entryStatusLabel(entry({ availability: "adapter_missing" })),
      "Adapter needed",
    );
    assert.equal(
      entryStatusLabel(entry({ availability: "adapter_outdated" })),
      "Update needed",
    );
    assert.equal(
      entryStatusLabel(entry({ availability: "cli_missing" })),
      "CLI needed",
    );
    assert.equal(
      entryStatusLabel(entry({ availability: "not_installed" })),
      "CLI needed",
    );
  });

  it("flags sign-in for available-but-logged-out", () => {
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          authStatus: { status: "logged_out" },
        }),
      ),
      "Sign-in needed",
    );
  });

  it("is silent for ready + logged in", () => {
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          authStatus: { status: "logged_in" },
        }),
      ),
      null,
    );
  });
});

// ── catalogPrimaryAction ─────────────────────────────────────────────────────

describe("catalogPrimaryAction", () => {
  it("no action for ready entries", () => {
    assert.deepEqual(
      catalogPrimaryAction(entry({ availability: "available" })),
      { kind: "none" },
    );
  });

  it("install for one-click installable", () => {
    assert.deepEqual(catalogPrimaryAction(entry({ canAutoInstall: true })), {
      kind: "install",
      label: "Install",
    });
  });

  it("update label for outdated adapters", () => {
    assert.deepEqual(
      catalogPrimaryAction(
        entry({ availability: "adapter_outdated", canAutoInstall: true }),
      ),
      { kind: "install", label: "Update" },
    );
  });

  it("docs fallback when auto-install is unavailable", () => {
    assert.deepEqual(
      catalogPrimaryAction(
        entry({ installInstructionsUrl: "https://example.com" }),
      ),
      { kind: "docs", label: "Open setup guide" },
    );
  });

  it("node-gated installs fall back to docs", () => {
    assert.deepEqual(
      catalogPrimaryAction(
        entry({
          canAutoInstall: true,
          nodeRequired: true,
          installInstructionsUrl: "https://example.com",
        }),
      ),
      { kind: "docs", label: "Open setup guide" },
    );
  });

  it("none when nothing is actionable", () => {
    assert.deepEqual(catalogPrimaryAction(entry({})), { kind: "none" });
  });
});
