/**
 * E2E spec for the consolidated Harnesses settings surface + Add-harnesses
 * catalog dialog.
 *
 * Covers:
 *  - Ready preset gets a row in "Your harnesses"; needs-setup preset does NOT
 *  - Needs-setup preset appears in the Add-harnesses catalog with status,
 *    curated description, docs link, and setup action
 *  - Catalog search filters the list
 *  - Toggling install does not reorder rows (stable order)
 *  - Add custom harness via catalog (name+command → save → row appears)
 *  - Advanced disclosure hides ID/args/env/docs/hint until opened
 *  - Edit preserves env vars (round-trip through definitionEnv boundary)
 *  - Same-ID edit replaces entry (no duplicate row)
 *  - Rename removes old row and shows new row
 *  - Delete success removes row; delete failure shows inline error
 *  - PATH badge: custom harness row shows Detected when available
 *  - Preset rows render bundled logos, never initials
 *  - Onboarding navigate: setup-page "More harnesses" click → Settings → Agents (F8)
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { passThroughBackupStep } from "../helpers/onboarding";

// ── Shared catalog fixtures ───────────────────────────────────────────────────

/** Hermes preset with availability "available" — earns a Your-harnesses row. */
const HERMES_AVAILABLE = {
  id: "hermes",
  label: "Hermes",
  avatar_url: "",
  availability: "available",
  command: "hermes-acp",
  binary_path: "/usr/local/bin/hermes-acp",
  default_args: [],
  mcp_command: null,
  install_hint: "Buzz talks to Hermes Agent through its hermes-acp command.",
  install_instructions_url: "https://hermes-agent.nousresearch.com",
  can_auto_install: false,
  requires_external_cli: true,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "unknown" },
  source: "preset",
} as const;

/** OpenClaw preset "not_installed" + no auto-install — catalog-only. */
const OPENCLAW_NOT_INSTALLED = {
  id: "openclaw",
  label: "OpenClaw",
  avatar_url: "",
  availability: "not_installed",
  command: "openclaw",
  binary_path: null,
  default_args: ["acp"],
  mcp_command: null,
  install_hint:
    "Buzz talks to OpenClaw through its ACP mode (openclaw acp), which relies on the OpenClaw Gateway daemon. Follow the setup guide to install both.",
  install_instructions_url: "https://docs.openclaw.ai/start/getting-started",
  can_auto_install: false,
  requires_external_cli: true,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "unknown" },
  source: "preset",
} as const;

/** Cursor preset — deliberately has NO bundled logo (brand assets not
 * licensed for redistribution). Must render the terminal glyph, never
 * initials. */
const CURSOR_AVAILABLE = {
  id: "cursor",
  label: "Cursor",
  avatar_url: "",
  availability: "available",
  command: "cursor-agent",
  binary_path: "/usr/local/bin/cursor-agent",
  default_args: [],
  mcp_command: null,
  install_hint: "Buzz talks to Cursor through the cursor-agent CLI's ACP mode.",
  install_instructions_url: "https://cursor.com/cli",
  can_auto_install: false,
  requires_external_cli: true,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "unknown" },
  source: "preset",
} as const;

/** Custom harness entry already persisted — always gets a row. */
function makeCustomEntry(
  overrides: {
    id?: string;
    label?: string;
    command?: string;
    availability?: "available" | "not_installed";
    definition_env?: Record<string, string>;
  } = {},
) {
  return {
    id: overrides.id ?? "my-custom-agent",
    label: overrides.label ?? "My Custom Agent",
    avatar_url: "",
    availability: overrides.availability ?? "not_installed",
    command: overrides.command ?? "my-custom-acp",
    binary_path:
      overrides.availability === "available"
        ? "/usr/local/bin/my-custom-acp"
        : null,
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url: "",
    can_auto_install: false,
    requires_external_cli: true,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "unknown" },
    source: "custom",
    definition_env: overrides.definition_env,
  };
}

// ── Navigation helpers ────────────────────────────────────────────────────────

/**
 * Open Settings → Agents through the normal UI path.
 * CI serves the app as a static SPA; direct navigation to /settings 404s
 * before the client router starts.
 */
async function openHarnessSettings(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-settings").click();
  await page.getByTestId("profile-popover-settings").click();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await page.getByTestId("settings-nav-agents").click();
  await expect(page.getByTestId("settings-harnesses")).toBeVisible({
    timeout: 10_000,
  });
}

/** Open the Add-harnesses catalog dialog from the settings surface. */
async function openCatalog(page: import("@playwright/test").Page) {
  await page.getByTestId("harness-add-button").click();
  await expect(page.getByTestId("harness-catalog-dialog")).toBeVisible();
}

/** Fill the custom harness form (all fields render inline). */
async function fillHarnessForm(
  page: import("@playwright/test").Page,
  values: {
    label: string;
    id?: string;
    command: string;
    env?: Array<{ key: string; value: string }>;
  },
) {
  await page.fill("#ch-label", values.label);
  await page.fill("#ch-command", values.command);
  if (values.id !== undefined) {
    await page.fill("#ch-id", values.id);
  }
  for (const pair of values.env ?? []) {
    await page.getByRole("button", { name: "Add env var" }).click();
    // Fill last appended row.
    const keyInputs = page.locator('input[placeholder="KEY"]');
    const valInputs = page.locator('input[placeholder="value"]');
    await keyInputs.last().fill(pair.key);
    await valInputs.last().fill(pair.value);
  }
}

// ── Your harnesses vs catalog split ──────────────────────────────────────────

test.describe("your harnesses split", () => {
  test("ready preset gets a row; needs-setup preset is catalog-only", async ({
    page,
  }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [HERMES_AVAILABLE, OPENCLAW_NOT_INSTALLED],
    });
    await openHarnessSettings(page);

    // Ready preset row with a Ready status chip.
    const hermesRow = page.getByTestId("doctor-runtime-hermes");
    await expect(hermesRow).toBeVisible();
    await expect(page.getByTestId("doctor-runtime-ready-hermes")).toHaveText(
      "Ready",
    );

    // Needs-setup preset must NOT render a row (and thus no Install button).
    await expect(page.getByTestId("doctor-runtime-openclaw")).toHaveCount(0);
    await expect(
      page.getByTestId("doctor-runtime-install-openclaw"),
    ).toHaveCount(0);
  });

  test("needs-setup preset appears in catalog with status, description, docs and setup action", async ({
    page,
  }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [HERMES_AVAILABLE, OPENCLAW_NOT_INSTALLED],
    });
    await openHarnessSettings(page);
    await openCatalog(page);

    // Needs-setup entries sort first, so OpenClaw is auto-selected.
    await expect(
      page.getByTestId("harness-catalog-list-item-openclaw"),
    ).toBeVisible();
    const detail = page.getByTestId("harness-catalog-detail-pane");
    await expect(detail).toContainText("OpenClaw");
    // Status chip.
    await expect(
      page.getByTestId("harness-catalog-status-openclaw"),
    ).toHaveText("CLI needed");
    // Curated one-liner (first-party-sourced category sentence).
    await expect(detail).toContainText(
      "A personal AI assistant that runs on your own devices.",
    );
    // Operational setup hint from runtime state.
    await expect(detail).toContainText("OpenClaw Gateway daemon");
    // Primary setup action under the header (no auto-install → setup guide).
    await expect(
      page.getByTestId("harness-catalog-setup-openclaw"),
    ).toBeVisible();

    // Technical details are visible by default.
    const technical = page.getByTestId("harness-catalog-technical-openclaw");
    await expect(technical).toBeVisible();
    await expect(technical).toContainText("openclaw");
    await expect(technical).toContainText("acp");
  });

  test("ready preset shows in catalog as Ready with no install action", async ({
    page,
  }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [HERMES_AVAILABLE, OPENCLAW_NOT_INSTALLED],
    });
    await openHarnessSettings(page);
    await openCatalog(page);

    // Ready entries live in the "Installed" accordion, collapsed by default.
    await expect(
      page.getByTestId("harness-catalog-list-item-hermes"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("harness-catalog-section-installed-count"),
    ).toHaveText("1");
    await page.getByTestId("harness-catalog-section-installed").click();

    await page.getByTestId("harness-catalog-list-item-hermes").click();
    const detail = page.getByTestId("harness-catalog-detail-pane");
    await expect(detail).toContainText("Ready");
    await expect(
      page.getByTestId("harness-catalog-install-hermes"),
    ).toHaveCount(0);
    await expect(page.getByTestId("harness-catalog-setup-hermes")).toHaveCount(
      0,
    );
  });

  test("catalog Update for an outdated adapter requires confirmation before installing", async ({
    page,
  }) => {
    // Wes's review blocker: Add runtimes → Setup → Update must honor the
    // same machine-wide replacement confirmation the runtime row shows —
    // never mutate straight from the catalog CTA.
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        HERMES_AVAILABLE,
        {
          ...OPENCLAW_NOT_INSTALLED,
          availability: "adapter_outdated",
          binary_path: "/usr/local/bin/openclaw",
          can_auto_install: true,
        },
      ],
    });
    await openHarnessSettings(page);
    await openCatalog(page);

    const installCalls = () =>
      page.evaluate(
        () =>
          (
            (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
              .__BUZZ_E2E_COMMANDS__ ?? []
          ).filter((command) => command === "install_acp_runtime").length,
      );

    await page.getByTestId("harness-catalog-list-item-openclaw").click();
    await expect(
      page.getByTestId("harness-catalog-status-openclaw"),
    ).toHaveText("Update needed");
    const updateButton = page.getByTestId("harness-catalog-install-openclaw");
    await expect(updateButton).toHaveText("Update");

    // Cancel path: clicking Update opens the warning, no mutation fires.
    await updateButton.click();
    const dialog = page.getByRole("alertdialog");
    await expect(dialog).toContainText("Update OpenClaw adapter?");
    await expect(dialog).toContainText(
      "This replaces the machine-wide openclaw adapter.",
    );
    // Generic runtimes must never get Codex's package copy.
    await expect(dialog).not.toContainText("codex-acp");
    expect(await installCalls()).toBe(0);
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(dialog).toHaveCount(0);
    expect(await installCalls()).toBe(0);

    // Confirm path: exactly one install fires after confirmation.
    await updateButton.click();
    await page.getByTestId("harness-catalog-confirm-update-openclaw").click();
    await expect(page.getByRole("alertdialog")).toHaveCount(0);
    await expect.poll(installCalls).toBe(1);
    // No duplicate mutation after the flow settles.
    expect(await installCalls()).toBe(1);
  });

  test("catalog search filters the list", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        HERMES_AVAILABLE,
        OPENCLAW_NOT_INSTALLED,
        CURSOR_AVAILABLE,
      ],
    });
    await openHarnessSettings(page);
    await openCatalog(page);

    await page.getByTestId("harness-catalog-search").fill("claw");
    await expect(
      page.getByTestId("harness-catalog-list-item-openclaw"),
    ).toBeVisible();
    await expect(
      page.getByTestId("harness-catalog-list-item-hermes"),
    ).toHaveCount(0);
    await expect(
      page.getByTestId("harness-catalog-list-item-cursor"),
    ).toHaveCount(0);
    // The custom-harness entry stays available regardless of the filter.
    await expect(
      page.getByTestId("harness-catalog-list-item-custom"),
    ).toBeVisible();
  });
});

// ── Preset logos in Your harnesses rows ──────────────────────────────────────

test("harness rows render bundled preset logos, not initials", async ({
  page,
}) => {
  await installMockBridge(page, {
    acpRuntimesCatalog: [HERMES_AVAILABLE, CURSOR_AVAILABLE],
  });
  await openHarnessSettings(page);

  // Preset rows must show the same bundled logo the catalog uses
  // (PRESET_LOGOS via RuntimeIcon), even though presets emit an empty
  // avatar_url (the no-remote-icon security line).
  const hermesLogo = page.getByTestId("doctor-runtime-logo-hermes");
  await expect(hermesLogo).toBeVisible();
  await expect(hermesLogo.locator("img")).toHaveAttribute(
    "src",
    "/harness-logos/hermes.png",
  );

  // Cursor renders its inline SVG mark (RUNTIME_MARKS, CC0 simple-icons
  // path) — an svg, never an img or initials.
  const cursorLogo = page.getByTestId("doctor-runtime-logo-cursor");
  await expect(cursorLogo).toBeVisible();
  await expect(cursorLogo.locator("svg")).toBeVisible();
  await expect(cursorLogo.locator("img")).not.toBeVisible();
  await expect(cursorLogo).not.toContainText("C");
});

// ── Custom harness add (via catalog) ─────────────────────────────────────────

test.describe("add custom harness", () => {
  test("catalog custom form saves and row appears in Your harnesses", async ({
    page,
  }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [HERMES_AVAILABLE, OPENCLAW_NOT_INSTALLED],
    });
    await openHarnessSettings(page);

    // No custom rows yet.
    await expect(
      page.getByTestId("doctor-runtime-my-custom-agent"),
    ).not.toBeVisible();

    await openCatalog(page);
    await page.getByTestId("harness-catalog-list-item-custom").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();

    // All fields render inline.
    await expect(page.locator("#ch-label")).toBeVisible();
    await expect(page.locator("#ch-command")).toBeVisible();
    await expect(page.locator("#ch-id")).toBeVisible();

    await fillHarnessForm(page, {
      label: "My Custom Agent",
      command: "my-custom-acp",
    });

    // ID auto-derives from the label.
    await expect(page.locator("#ch-id")).toHaveValue("my-custom-agent");

    await page
      .getByTestId("custom-harness-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();

    // Dialog closes; row appears in Your harnesses.
    await expect(page.getByTestId("harness-catalog-dialog")).not.toBeVisible();
    await expect(
      page.getByTestId("doctor-runtime-my-custom-agent"),
    ).toBeVisible({ timeout: 5_000 });
  });

  test("edit preserves env vars (definitionEnv round-trip)", async ({
    page,
  }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        HERMES_AVAILABLE,
        OPENCLAW_NOT_INSTALLED,
        makeCustomEntry({ definition_env: { MY_API_KEY: "sk-test" } }),
      ],
    });
    await openHarnessSettings(page);

    // Open edit form for the existing custom entry via the ••• menu.
    await expect(
      page.getByTestId("doctor-runtime-my-custom-agent"),
    ).toBeVisible();
    await page.getByTestId("doctor-runtime-menu-my-custom-agent").click();
    await page.getByTestId("custom-harness-edit-my-custom-agent").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();

    // Existing env vars must be pre-populated.
    await expect(page.locator('input[placeholder="KEY"]').first()).toHaveValue(
      "MY_API_KEY",
    );
    await expect(
      page.locator('input[placeholder="value"]').first(),
    ).toHaveValue("sk-test");
  });

  test("same-ID edit replaces row — no duplicate", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        HERMES_AVAILABLE,
        OPENCLAW_NOT_INSTALLED,
        makeCustomEntry({ label: "V1 Label" }),
      ],
    });
    await openHarnessSettings(page);

    // Edit, keep same ID, change label.
    await page.getByTestId("doctor-runtime-menu-my-custom-agent").click();
    await page.getByTestId("custom-harness-edit-my-custom-agent").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();
    await page.fill("#ch-label", "V2 Label");
    await page
      .getByTestId("custom-harness-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();

    // Exactly one row with the same ID; label updated.
    const rows = page.locator('[data-testid="doctor-runtime-my-custom-agent"]');
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText("V2 Label");
  });

  test("rename removes old row and inserts new row", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        HERMES_AVAILABLE,
        OPENCLAW_NOT_INSTALLED,
        makeCustomEntry({ id: "old-harness", label: "Old" }),
      ],
    });
    await openHarnessSettings(page);

    await page.getByTestId("doctor-runtime-menu-old-harness").click();
    await page.getByTestId("custom-harness-edit-old-harness").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();
    await fillHarnessForm(page, {
      label: "New Harness",
      id: "new-harness",
      command: "my-custom-acp",
    });
    await page
      .getByTestId("custom-harness-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();

    // Old row gone; new row present.
    await expect(
      page.getByTestId("doctor-runtime-old-harness"),
    ).not.toBeVisible({ timeout: 5_000 });
    await expect(page.getByTestId("doctor-runtime-new-harness")).toBeVisible({
      timeout: 5_000,
    });
  });
});

// ── Delete flow ───────────────────────────────────────────────────────────────

test.describe("delete custom harness", () => {
  test("delete success removes the row", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        HERMES_AVAILABLE,
        OPENCLAW_NOT_INSTALLED,
        makeCustomEntry(),
      ],
    });
    await openHarnessSettings(page);

    // Enter confirm-delete mode via the ••• menu.
    await page.getByTestId("doctor-runtime-menu-my-custom-agent").click();
    await page.getByTestId("custom-harness-delete-my-custom-agent").click();
    // Blast-radius warning + confirm button must appear.
    await expect(
      page.getByTestId("custom-harness-delete-warning-my-custom-agent"),
    ).toBeVisible();
    await expect(
      page.getByTestId("custom-harness-delete-confirm-my-custom-agent"),
    ).toBeVisible();
    await page
      .getByTestId("custom-harness-delete-confirm-my-custom-agent")
      .click();

    // Row disappears after successful delete.
    await expect(
      page.getByTestId("doctor-runtime-my-custom-agent"),
    ).not.toBeVisible({ timeout: 5_000 });
  });

  test("delete failure shows inline error and keeps the row", async ({
    page,
  }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        HERMES_AVAILABLE,
        OPENCLAW_NOT_INSTALLED,
        makeCustomEntry(),
      ],
      deleteCustomHarnessError: "permission denied: could not remove file",
    });
    await openHarnessSettings(page);

    await page.getByTestId("doctor-runtime-menu-my-custom-agent").click();
    await page.getByTestId("custom-harness-delete-my-custom-agent").click();
    await page
      .getByTestId("custom-harness-delete-confirm-my-custom-agent")
      .click();

    // Error text visible; row still present.
    await expect(
      page.getByText("permission denied: could not remove file"),
    ).toBeVisible({ timeout: 5_000 });
    await expect(
      page.getByTestId("doctor-runtime-my-custom-agent"),
    ).toBeVisible();
  });
});

// ── Custom harness row readiness ─────────────────────────────────────────────

test("available custom harness row shows a Ready chip", async ({ page }) => {
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      HERMES_AVAILABLE,
      OPENCLAW_NOT_INSTALLED,
      makeCustomEntry({ availability: "available" }),
    ],
  });
  await openHarnessSettings(page);

  const row = page.getByTestId("doctor-runtime-my-custom-agent");
  await expect(row).toBeVisible();
  await expect(
    page.getByTestId("doctor-runtime-ready-my-custom-agent"),
  ).toHaveText("Ready");
});

test("not-ready custom harness row shows status, no install action", async ({
  page,
}) => {
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      HERMES_AVAILABLE,
      OPENCLAW_NOT_INSTALLED,
      makeCustomEntry(),
    ],
  });
  await openHarnessSettings(page);

  const row = page.getByTestId("doctor-runtime-my-custom-agent");
  await expect(row).toBeVisible();
  await expect(
    page.getByTestId("doctor-runtime-status-my-custom-agent"),
  ).toHaveText("CLI needed");
  // A row that setup can't fix with one click renders no Install button (and
  // is not ready).
  await expect(
    page.getByTestId("doctor-runtime-install-my-custom-agent"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("doctor-runtime-ready-my-custom-agent"),
  ).toHaveCount(0);
});

// ── F8: onboarding navigate-after-complete ────────────────────────────────────
//
// Verifies the parent-owned route intent introduced in B-8:
//   1. User reaches the machine-onboarding setup page.
//   2. Clicks "More harnesses" (onboarding-setup-more-harnesses).
//   3. App completes onboarding and immediately navigates to Settings → Agents.
//
// This test exercises the real App.tsx effect that gates router.navigate() on
// machine.stage === "ready", which the pure-logic tests in
// postOnboardingNav.test.mjs cannot cover (they simulate the predicate, not
// the real render path).

test("onboarding setup More-harnesses click navigates to Settings → Agents", async ({
  page,
}) => {
  // Start with a fresh machine (no machine-onboarding-complete flag).
  // skipCommunitySeed: true so the user goes through machine onboarding.
  // skipOnboardingSeed: true so the community/identity banner doesn't appear.
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  // Seed a community stamped with a *foreign* pubkey. This is the only shape
  // that satisfies both preconditions of this test at once:
  //   - machine onboarding must still run, so the community must NOT vouch for
  //     the active identity (migrateMachineOnboardingCompletion only accepts a
  //     community whose recorded pubkey matches — see machineOnboarding.ts:70).
  //   - after onboarding completes, useCommunityInit must NOT report
  //     needsSetup, or App.tsx:499 renders WelcomeSetup instead of the router
  //     and the navigation lands on a screen that has no settings tree.
  // The default seed vouches (it uses the active pubkey) and skipping it
  // entirely leaves zero communities, so neither default gets there.
  await page.addInitScript(() => {
    const communityId = "e2e-default-community";
    window.localStorage.setItem(
      "buzz-communities",
      JSON.stringify([
        {
          id: communityId,
          name: "E2E Test",
          relayUrl: "ws://127.0.0.1:7777",
          pubkey: "f".repeat(64),
          addedAt: new Date().toISOString(),
        },
      ]),
    );
    window.localStorage.setItem("buzz-active-community-id", communityId);
  });
  await page.goto("/");

  // Reach setup by creating a new identity key and continuing past the
  // created-key page without opening the optional backup options.
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await passThroughBackupStep(page);

  // Now on the setup page.
  await expect(
    page.getByRole("heading", { name: "Set up your agent harnesses" }),
  ).toBeVisible({ timeout: 10_000 });

  // Click the "More harnesses" link — fires navigateToAgentSettings.
  await page.getByTestId("onboarding-setup-more-harnesses").click();

  // After onboarding completes + router mounts, the app must land on
  // Settings → Agents (consolidated harnesses section visible).
  await expect(page.getByTestId("settings-harnesses")).toBeVisible({
    timeout: 15_000,
  });
});
