import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  adapterUpdateWarning,
  catalogDialogEntries,
  catalogPrimaryAction,
  entryLoginHint,
  entryStatusLabel,
  filterCatalogEntries,
  groupCatalogEntries,
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
    runtimeReadiness: "ready",
    setupFields: [],
    canConnectAccount: false,
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

// ── groupCatalogEntries ──────────────────────────────────────────────────────

describe("groupCatalogEntries", () => {
  it("splits operationally ready entries from setup-required entries", () => {
    const entries = [
      entry({ id: "needs-cli", availability: "cli_missing" }),
      entry({ id: "ready", availability: "available" }),
      entry({
        id: "needs-model",
        availability: "available",
        runtimeReadiness: "model_unavailable",
      }),
    ];
    const groups = groupCatalogEntries(entries);
    assert.deepEqual(
      groups.setup.map((e) => e.id),
      ["needs-cli", "needs-model"],
    );
    assert.deepEqual(
      groups.installed.map((e) => e.id),
      ["ready"],
    );
  });

  it("preserves input order within each group", () => {
    const entries = [
      entry({ id: "b", availability: "available" }),
      entry({ id: "a", availability: "available" }),
    ];
    assert.deepEqual(
      groupCatalogEntries(entries).installed.map((e) => e.id),
      ["b", "a"],
    );
  });

  it("handles empty input", () => {
    assert.deepEqual(groupCatalogEntries([]), { setup: [], installed: [] });
  });
});

// ── filterCatalogEntries ─────────────────────────────────────────────────────

describe("filterCatalogEntries", () => {
  const entries = [
    entry({ id: "kimi", label: "Kimi Code", command: "kimi" }),
    entry({ id: "amp", label: "Amp", command: "amp" }),
    entry({ id: "ompk", label: "Oh My PK", command: "ompk" }),
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
      filterCatalogEntries(entries, "ompk").map((e) => e.id),
      ["ompk"],
    );
  });

  it("returns empty for no match", () => {
    assert.equal(filterCatalogEntries(entries, "zzz").length, 0);
  });
});

// ── stableRowOrder ───────────────────────────────────────────────────────────

describe("stableRowOrder", () => {
  it("initial order: priority builtins, then ready, then alpha", () => {
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
        label: "Pkzz",
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

  it("keeps previous relative order when availability changes (no reorder on install)", () => {
    const before = ["buzz-agent", "zeta", "off"];
    const entries = [
      entry({ id: "off", label: "Aardvark", availability: "available" }), // just installed
      entry({ id: "zeta", label: "Zeta", availability: "available" }),
      entry({
        id: "buzz-agent",
        label: "Pkzz",
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
      entry({ id: "buzz-agent", label: "Pkzz", availability: "available" }),
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
  it("names connection setup without claiming the binary is ready", () => {
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          runtimeReadiness: "configuration_required",
        }),
      ),
      "Connection setup needed",
    );
  });
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

  it("maps generic Rust readiness states for available entries", () => {
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          runtimeReadiness: "authentication_required",
        }),
      ),
      "Sign-in needed",
    );
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          runtimeReadiness: "model_unavailable",
        }),
      ),
      "Model setup needed",
    );
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          runtimeReadiness: "unknown",
        }),
      ),
      "Status unavailable",
    );
    assert.equal(
      entryStatusLabel(
        entry({
          availability: "available",
          authStatus: { status: "unknown" },
          runtimeReadiness: "ready",
        }),
      ),
      null,
    );
  });
});

// ── entryLoginHint ───────────────────────────────────────────────────────────

describe("entryLoginHint", () => {
  const hint = "Sign in with `ompk auth-broker login anthropic`.";

  it("shows the hint only when authentication is required", () => {
    assert.equal(
      entryLoginHint(
        entry({
          availability: "available",
          authStatus: { status: "unknown" },
          runtimeReadiness: "authentication_required",
          loginHint: hint,
        }),
      ),
      hint,
    );
  });

  it("suppresses login copy for ready/keyless and model-setup runtimes", () => {
    for (const runtimeReadiness of ["ready", "model_unavailable", "unknown"]) {
      assert.equal(
        entryLoginHint(
          entry({
            availability: "available",
            authStatus: { status: "unknown" },
            runtimeReadiness,
            loginHint: hint,
          }),
        ),
        null,
      );
    }
  });

  it("stays silent while the CLI is missing — install copy owns that row", () => {
    assert.equal(
      entryLoginHint(entry({ availability: "not_installed", loginHint: hint })),
      null,
    );
  });

  it("treats an absent or blank hint as nothing to say", () => {
    assert.equal(entryLoginHint(entry({ availability: "available" })), null);
    assert.equal(
      entryLoginHint(entry({ availability: "available", loginHint: "   " })),
      null,
    );
  });
});

// ── adapterUpdateWarning ─────────────────────────────────────────────────────

describe("adapterUpdateWarning", () => {
  it("keeps the codex-specific machine-wide caveat for codex", () => {
    const copy = adapterUpdateWarning(
      entry({ id: "codex", label: "Codex", command: "codex-acp" }),
    );
    assert.match(copy, /codex-acp/);
    assert.match(copy, /@zed-industries\/codex-acp@0\.16\.0/);
  });

  it("never leaks codex package copy into other runtimes", () => {
    const copy = adapterUpdateWarning(
      entry({
        id: "claude",
        label: "Claude Code",
        command: "claude-agent-acp",
      }),
    );
    assert.match(copy, /claude-agent-acp/);
    assert.doesNotMatch(copy, /codex/i);
    assert.doesNotMatch(copy, /zed-industries/);
  });

  it("falls back to the label when the command is missing", () => {
    const copy = adapterUpdateWarning(
      entry({ id: "mystery", label: "Mystery Harness", command: null }),
    );
    assert.match(copy, /Mystery Harness/);
    assert.doesNotMatch(copy, /codex/i);
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

  it("opens setup guidance for installed but non-ready entries", () => {
    assert.deepEqual(
      catalogPrimaryAction(
        entry({
          availability: "available",
          runtimeReadiness: "model_unavailable",
          canAutoInstall: true,
          installInstructionsUrl: "https://example.com/setup",
        }),
      ),
      { kind: "docs", label: "Setup guide" },
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
      { kind: "docs", label: "Setup guide" },
    );
  });

  it("docs fallback labels download pages honestly", () => {
    assert.deepEqual(
      catalogPrimaryAction(
        entry({ installInstructionsUrl: "https://cursor.com/downloads" }),
      ),
      { kind: "docs", label: "Download page" },
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
      { kind: "docs", label: "Setup guide" },
    );
  });

  it("none when nothing is actionable", () => {
    assert.deepEqual(catalogPrimaryAction(entry({})), { kind: "none" });
  });
});
