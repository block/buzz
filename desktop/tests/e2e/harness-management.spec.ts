/**
 * E2E spec for the Bring-Your-Own-Harness management UI.
 *
 * Covers:
 *  - Preset gallery renders with Detected badge for an available preset
 *  - Preset gallery renders without badge / with install link for a missing preset
 *  - Add custom harness (form → save → row appears in list)
 *  - Edit preserves env vars (round-trip through definitionEnv boundary)
 *  - Same-ID edit replaces entry (no duplicate row)
 *  - Rename removes old row and shows new row
 *  - Delete success removes row
 *  - Delete failure shows error inline (error-injection knob)
 *  - PATH badge: custom harness row shows Detected when availability === "available"
 *  - Onboarding navigate: setup-page "More harnesses" click → Settings → Agents (F8)
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// ── Shared catalog fixtures ───────────────────────────────────────────────────

/** Hermes preset with availability "available" — renders the Detected badge. */
const HERMES_AVAILABLE = {
  id: "hermes",
  label: "Hermes",
  avatar_url: "",
  availability: "available",
  command: "hermes-acp",
  binary_path: "/usr/local/bin/hermes-acp",
  default_args: [],
  mcp_command: null,
  install_hint: "Install Hermes Agent from hermes-agent.nousresearch.com.",
  install_instructions_url: "https://hermes-agent.nousresearch.com",
  can_auto_install: false,
  requires_external_cli: true,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "unknown" },
  source: "preset",
} as const;

/** OpenClaw preset with availability "not_installed" — renders install link. */
const OPENCLAW_NOT_INSTALLED = {
  id: "openclaw",
  label: "OpenClaw",
  avatar_url: "",
  availability: "not_installed",
  command: "openclaw",
  binary_path: null,
  default_args: ["acp"],
  mcp_command: null,
  install_hint: "Install OpenClaw: npm install -g openclaw@latest.",
  install_instructions_url: "https://docs.openclaw.ai/start/getting-started",
  can_auto_install: false,
  requires_external_cli: true,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "unknown" },
  source: "preset",
} as const;

/** Cursor preset — deliberately has NO bundled logo (brand assets not
 * licensed for redistribution; see FALLBACK_ONLY_PRESETS). Must render the
 * terminal glyph, never initials. */
const CURSOR_AVAILABLE = {
  id: "cursor",
  label: "Cursor",
  avatar_url: "",
  availability: "available",
  command: "cursor-agent",
  binary_path: "/usr/local/bin/cursor-agent",
  default_args: [],
  mcp_command: null,
  install_hint: "Install Cursor CLI from cursor.com.",
  install_instructions_url: "https://cursor.com/cli",
  can_auto_install: false,
  requires_external_cli: true,
  underlying_cli_path: null,
  node_required: false,
  auth_status: { status: "unknown" },
  source: "preset",
} as const;

/** Custom harness entry already persisted — shown in the custom list. */
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
  await expect(page.getByTestId("settings-harness-management")).toBeVisible({
    timeout: 10_000,
  });
}

/**
 * Fill and submit the custom harness add/edit form.
 * Caller must have the form visible before calling.
 */
async function fillHarnessForm(
  page: import("@playwright/test").Page,
  values: {
    label: string;
    id: string;
    command: string;
    env?: Array<{ key: string; value: string }>;
  },
) {
  await page.fill("#ch-label", values.label);
  // ID may auto-derive; overwrite it.
  await page.fill("#ch-id", values.id);
  await page.fill("#ch-command", values.command);
  for (const pair of values.env ?? []) {
    await page.getByRole("button", { name: "Add env var" }).click();
    // Fill last appended row.
    const keyInputs = page.locator('input[placeholder="KEY"]');
    const valInputs = page.locator('input[placeholder="value"]');
    await keyInputs.last().fill(pair.key);
    await valInputs.last().fill(pair.value);
  }
}

// ── Preset gallery ────────────────────────────────────────────────────────────

test.describe("preset gallery", () => {
  test("detected preset shows Detected badge", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [HERMES_AVAILABLE, OPENCLAW_NOT_INSTALLED],
    });
    await openHarnessSettings(page);

    const hermesCard = page.getByTestId("harness-preset-hermes");
    await expect(hermesCard).toBeVisible();
    await expect(hermesCard.getByText("Detected")).toBeVisible();
    // Not-installed preset must NOT show Detected badge.
    const openclawCard = page.getByTestId("harness-preset-openclaw");
    await expect(openclawCard).toBeVisible();
    await expect(openclawCard.getByText("Detected")).not.toBeVisible();
  });

  test("not-installed preset shows Install link, not Detected badge", async ({
    page,
  }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [HERMES_AVAILABLE, OPENCLAW_NOT_INSTALLED],
    });
    await openHarnessSettings(page);

    const openclawCard = page.getByTestId("harness-preset-openclaw");
    await expect(openclawCard).toBeVisible();
    await expect(openclawCard.getByText("Install")).toBeVisible();
    await expect(openclawCard.getByText("Detected")).not.toBeVisible();
  });
});

// ── Preset logos in the Agent runtimes list ──────────────────────────────────

test("Agent runtimes rows render bundled preset logos, not initials", async ({
  page,
}) => {
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      HERMES_AVAILABLE,
      OPENCLAW_NOT_INSTALLED,
      CURSOR_AVAILABLE,
    ],
  });
  await openHarnessSettings(page);

  // Preset rows in "Agent runtimes" must show the same bundled logo the
  // preset gallery uses (PRESET_LOGOS via RuntimeIcon), even though presets
  // emit an empty avatar_url (the no-remote-icon security line).
  for (const [id, file] of [
    ["hermes", "/harness-logos/hermes.png"],
    ["openclaw", "/harness-logos/openclaw.svg"],
  ] as const) {
    const logo = page.getByTestId(`doctor-runtime-logo-${id}`);
    await expect(logo).toBeVisible();
    await expect(logo.locator("img")).toHaveAttribute("src", file);
  }

  // Cursor has no bundled logo (licensing) — it must fall through to
  // RuntimeIcon's terminal glyph like the preset gallery, not initials.
  const cursorLogo = page.getByTestId("doctor-runtime-logo-cursor");
  await expect(cursorLogo).toBeVisible();
  await expect(cursorLogo.locator("svg")).toBeVisible();
  await expect(cursorLogo.locator("img")).not.toBeVisible();
  await expect(cursorLogo).not.toContainText("C");
});

// ── Custom harness add ────────────────────────────────────────────────────────

test.describe("add custom harness", () => {
  test("form saves and row appears in list", async ({ page }) => {
    await installMockBridge(page, {
      acpRuntimesCatalog: [HERMES_AVAILABLE, OPENCLAW_NOT_INSTALLED],
    });
    await openHarnessSettings(page);

    // No custom rows yet.
    // The list container may exist but have no row children.
    await expect(
      page.getByTestId("custom-harness-row-my-custom-agent"),
    ).not.toBeVisible();

    // Open the add form.
    await page.getByTestId("harness-add-custom-button").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();

    await fillHarnessForm(page, {
      label: "My Custom Agent",
      id: "my-custom-agent",
      command: "my-custom-acp",
    });

    // Submit.
    await page
      .getByTestId("custom-harness-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();

    // Row must appear in the list after save.
    await expect(
      page.getByTestId("custom-harness-row-my-custom-agent"),
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

    // Open edit form for the existing custom entry.
    await expect(
      page.getByTestId("custom-harness-row-my-custom-agent"),
    ).toBeVisible();
    await page.getByTestId("custom-harness-edit-my-custom-agent").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();

    // Env KEY and value must be pre-populated.
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
    await page.getByTestId("custom-harness-edit-my-custom-agent").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();
    await page.fill("#ch-label", "V2 Label");
    await page
      .getByTestId("custom-harness-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();

    // Exactly one row with the same ID; label updated.
    const rows = page.locator(
      '[data-testid^="custom-harness-row-my-custom-agent"]',
    );
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

    await page.getByTestId("custom-harness-edit-old-harness").click();
    await expect(page.getByTestId("custom-harness-form")).toBeVisible();
    await page.fill("#ch-label", "New Harness");
    await page.fill("#ch-id", "new-harness");
    await page
      .getByTestId("custom-harness-form")
      .getByRole("button", { name: "Save", exact: true })
      .click();

    // Old row gone; new row present.
    await expect(
      page.getByTestId("custom-harness-row-old-harness"),
    ).not.toBeVisible({ timeout: 5_000 });
    await expect(
      page.getByTestId("custom-harness-row-new-harness"),
    ).toBeVisible({ timeout: 5_000 });
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

    // Enter confirm-delete mode.
    await page.getByTestId("custom-harness-delete-my-custom-agent").click();
    // The confirm button must appear.
    await expect(
      page.getByTestId("custom-harness-delete-confirm-my-custom-agent"),
    ).toBeVisible();
    await page
      .getByTestId("custom-harness-delete-confirm-my-custom-agent")
      .click();

    // Row disappears after successful delete.
    await expect(
      page.getByTestId("custom-harness-row-my-custom-agent"),
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

    await page.getByTestId("custom-harness-delete-my-custom-agent").click();
    await page
      .getByTestId("custom-harness-delete-confirm-my-custom-agent")
      .click();

    // Error text visible; row still present.
    await expect(
      page.getByText("permission denied: could not remove file"),
    ).toBeVisible({ timeout: 5_000 });
    await expect(
      page.getByTestId("custom-harness-row-my-custom-agent"),
    ).toBeVisible();
  });
});

// ── PATH badge on custom harness row ─────────────────────────────────────────

test("custom harness row shows Detected badge when command is on PATH", async ({
  page,
}) => {
  await installMockBridge(page, {
    acpRuntimesCatalog: [
      HERMES_AVAILABLE,
      OPENCLAW_NOT_INSTALLED,
      makeCustomEntry({ availability: "available" }),
    ],
  });
  await openHarnessSettings(page);

  const row = page.getByTestId("custom-harness-row-my-custom-agent");
  await expect(row).toBeVisible();
  await expect(row.getByText("Detected")).toBeVisible();
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

  // Reach the setup page: create a new identity key → skip backup step.
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("onboarding-next").click();

  // Now on the setup page.
  await expect(
    page.getByRole("heading", { name: "Set up your agent harnesses" }),
  ).toBeVisible({ timeout: 10_000 });

  // Click the "More harnesses" link — fires navigateToAgentSettings.
  await page.getByTestId("onboarding-setup-more-harnesses").click();

  // After onboarding completes + router mounts, the app must land on
  // Settings → Agents (harness management section visible).
  await expect(page.getByTestId("settings-harness-management")).toBeVisible({
    timeout: 15_000,
  });
});
