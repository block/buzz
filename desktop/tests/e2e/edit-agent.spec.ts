import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const BAKED_DEFAULTS = [
  { key: "BUZZ_AGENT_PROVIDER", value: "anthropic", masked: false },
  {
    key: "BUZZ_AGENT_MODEL",
    value: "claude-opus-4-8",
    masked: false,
  },
  { key: "BUZZ_AGENT_THINKING_EFFORT", value: "high", masked: false },
  { key: "ANTHROPIC_API_KEY", value: "sk-ant-baked-test", masked: true },
];

// Edit-agent dialog coverage (Phase 1B.3b-pre). Written against TODAY'S
// EditAgentDialog, before the B3b re-host, so the re-host is guarded by a
// pre-existing spec rather than one written alongside it.
//
// Mock-boundary caveat: the e2eBridge `update_managed_agent` handler echoes
// name/model/systemPrompt/envVars/respondTo/respondToAllowlist into the
// mock store — it does NOT
// model the diff-based partial-update wire semantics (change-detected-or-omit,
// tri-state provider, harnessOverride derivation), and it ignores
// agentCommand/harnessOverride entirely. This spec therefore pins UI behavior
// (open → edit → save → persisted in UI), not wire semantics. The inherit
// toggle is not reachable here at all (see the routing pin below) — its
// behavior is covered by B3b's component-level pinning test (inherit-toggle
// → gate → submit); wire semantics stay component-test territory
// (personaRuntimeModel.test.mjs).

// Tyler's pubkey maps to gooseSurface in the mock bridge (runtimeId "goose"),
// which supports LLM provider selection — same seed the readiness-screenshot
// spec uses for its edit-dialog shot.
const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const AGENT_NAME = "Tyler Agent";
const PERSONA_ID = "persona-edit-e2e";

/**
 * Open the Edit Agent dialog for the seeded managed agent via the profile
 * panel (agents view → agent card → Edit quick action) — EditAgentDialog's
 * only mount path.
 */
async function openEditDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();

  const agentButton = page.getByRole("button", {
    name: `${AGENT_NAME} agent profile`,
  });
  await expect(agentButton).toBeVisible({ timeout: 10_000 });
  await agentButton.click();

  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("user-profile-edit-agent").click();

  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
    timeout: 10_000,
  });
  // Provider field visible = runtime catalog loaded and form settled.
  await expect(page.locator("#edit-agent-llm-provider")).toBeVisible({
    timeout: 10_000,
  });
}

/**
 * Pick an option from a PersonaDropdownField (menu-based, not a native
 * <select> — Create's fields are selects, Edit's are not).
 */
async function pickDropdownOption(
  page: import("@playwright/test").Page,
  triggerId: string,
  optionName: string | RegExp,
) {
  await page.locator(`#${triggerId}`).click();
  await page.getByRole("menuitemradio", { name: optionName }).click();
}

/**
 * Pick an option from a PersonaModelCombobox (popover-based combobox with
 * <button> items, not a PersonaDropdownField with menuitemradio items).
 */
async function pickModelComboboxOption(
  page: import("@playwright/test").Page,
  triggerId: string,
  optionName: string | RegExp,
) {
  await page.locator(`#${triggerId}`).click();
  // Items in PersonaModelCombobox are plain <button> elements inside the popover.
  // Query at page level because the popover is portaled outside the dialog.
  await page.getByRole("button", { name: optionName }).click();
}

test.describe("agent definition dialog", () => {
  test("owner-only-access build shows disabled agent access with an explanation", async ({
    page,
  }) => {
    await installMockBridge(page, {
      ownerOnlyAccessBuild: true,
      bakedBuildEnv: BAKED_DEFAULTS,
    });
    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    await page.getByTestId("new-agent-card").click();

    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Advanced", exact: true }).click();

    await expect(dialog.getByTestId("agent-respond-to")).toBeVisible();
    await expect(dialog.locator("#agent-respond-to")).toBeDisabled();
    await expect(dialog.locator("#agent-respond-to")).toContainText(
      "Only me (default)",
    );
    await expect(
      dialog.getByTestId("agent-respond-to-disabled-reason"),
    ).toHaveText("This build disallows changing this setting.");
  });
});

test.describe("edit agent dialog", () => {
  test("owner-only-access build shows a disabled owner-only access control with an explanation", async ({
    page,
  }) => {
    await installMockBridge(page, {
      ownerOnlyAccessBuild: true,
      bakedBuildEnv: BAKED_DEFAULTS,
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
          respondTo: "anyone",
        },
      ],
    });

    await openEditDialog(page);

    const accessControl = page.getByTestId("agent-respond-to");
    await expect(accessControl).toBeVisible();
    await expect(page.locator("#agent-respond-to")).toBeDisabled();
    await expect(page.locator("#agent-respond-to")).toContainText(
      "Only me (default)",
    );
    await expect(
      page.getByTestId("agent-respond-to-disabled-reason"),
    ).toHaveText("This build disallows changing this setting.");
  });

  test("OSS build keeps the managed-agent access control", async ({ page }) => {
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    await expect(page.getByTestId("agent-respond-to")).toBeVisible();
  });

  test("definition-only context renders access control and parallelism (rows 9–10 D-owned)", async ({
    page,
  }) => {
    // Rows 9–10: in definition-only context (zero-instance definition), access/parallelism
    // are D-owned and must be rendered. This test proves Artifact 4 access-field
    // completeness for the definition-only route (R5 — definition-only edit from library).
    const DEFONLY_PERSONA_ID = "persona-defonly-access-e2e";
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      personas: [
        {
          id: DEFONLY_PERSONA_ID,
          displayName: "Definition Only Agent",
          systemPrompt: "No instances here.",
          respondTo: "owner-only",
        },
      ],
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();

    // Navigate to the definition-edit route via the definitions library button.
    const defButton = page.getByRole("button", {
      name: "Definition Only Agent agent profile",
    });
    await expect(defButton).toBeVisible({ timeout: 10_000 });
    await defButton.click();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });

    // Rows 9–10 Artifact 4: access control IS visible in definition-only context.
    await expect(page.getByTestId("agent-respond-to")).toBeVisible({
      timeout: 10_000,
    });
    // D-owned parallelism field rendered.
    await expect(page.locator("#edit-agent-parallelism")).toBeVisible({
      timeout: 10_000,
    });
  });

  test("edits the agent name and persists it across a dialog reopen", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    const nameInput = page.locator("#edit-agent-name");
    await expect(nameInput).toHaveValue(AGENT_NAME);
    await nameInput.fill("Tyler Agent Renamed");

    await page.getByTestId("edit-agent-dialog-submit").click();
    await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();

    // Reopen: the dialog re-reads the managed-agents store, proving the save
    // survived the dialog lifecycle rather than living in local state. (The
    // panel HEADER is not asserted — it renders the relay profile name, which
    // the update path does not touch.)
    await page.getByTestId("user-profile-edit-agent").click();
    await expect(page.locator("#edit-agent-name")).toHaveValue(
      "Tyler Agent Renamed",
      { timeout: 10_000 },
    );
  });

  test("changes the model via custom entry and persists it", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    // Pick a provider so model discovery has a scope, then set a custom model.
    await pickDropdownOption(page, "edit-agent-llm-provider", "Anthropic");
    await pickModelComboboxOption(page, "edit-agent-model", "Custom model...");
    await page.locator("#edit-agent-custom-model").fill("claude-opus-4-5");
    // Anthropic requires a credential before save unlocks.
    await page.getByLabel("Anthropic API Key").fill("sk-test-edit-agent-e2e");

    const submit = page.getByTestId("edit-agent-dialog-submit");
    await expect(submit).toBeEnabled({ timeout: 10_000 });
    await submit.click();
    await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();

    await page.getByTestId("user-profile-edit-agent").click();
    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });
    // Custom model round-trips: the reopened dialog shows it in the custom
    // input (the discovered-model lists don't contain it).
    await expect(page.locator("#edit-agent-custom-model")).toHaveValue(
      "claude-opus-4-5",
      { timeout: 10_000 },
    );
  });

  test("keeps the custom command visible without opening Advanced", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    const advanced = page.getByRole("button", {
      name: "Advanced",
      exact: true,
    });
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await pickDropdownOption(page, "edit-agent-runtime", "Custom command");
    await expect(page.locator("#edit-agent-command")).toBeVisible();
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
  });

  test("marks a missing advanced credential without opening Advanced", async ({
    page,
  }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    const advanced = page.getByRole("button", {
      name: "Advanced",
      exact: true,
    });
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await pickDropdownOption(page, "edit-agent-llm-provider", "Databricks v2");
    await expect(advanced).toHaveAttribute("aria-expanded", "false");
    await expect(
      page.getByTestId("edit-agent-advanced-required-badge"),
    ).toHaveText("Required");
    await expect(page.getByTestId("edit-agent-dialog-submit")).toBeDisabled();

    await advanced.click();
    await expect(page.getByLabel("Value for DATABRICKS_HOST")).toBeVisible();
  });

  test("shows baked defaults in the instance editor", async ({ page }) => {
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    await expect(page.locator("#edit-agent-llm-provider")).toHaveText(
      "Anthropic (inherited from build)",
    );
    await expect(page.locator("#edit-agent-model")).toHaveText(
      "Inherit build default (claude-opus-4-8)",
    );
    const defaults = page.getByTestId("agent-ai-defaults-notice");
    await expect(
      defaults.getByText("Anthropic", { exact: true }),
    ).toBeVisible();
    await expect(
      defaults.getByText("claude-opus-4-8", { exact: true }),
    ).toBeVisible();
  });

  test("explicit global defaults override baked labels in the instance editor", async ({
    page,
  }) => {
    await installMockBridge(page, {
      bakedBuildEnv: BAKED_DEFAULTS,
      globalAgentConfig: {
        provider: "anthropic",
        model: "claude-opus-4-5",
        env_vars: { BUZZ_AGENT_THINKING_EFFORT: "low" },
      },
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
    });

    await openEditDialog(page);

    await expect(page.locator("#edit-agent-llm-provider")).toHaveText(
      "Use agent defaults (anthropic)",
    );
    await expect(page.locator("#edit-agent-model")).toHaveText(
      "Use agent defaults (claude-opus-4-5)",
    );
    const defaults = page.getByTestId("agent-ai-defaults-notice");
    await expect(
      defaults.getByText("Anthropic", { exact: true }),
    ).toBeVisible();
    await expect(
      defaults.getByText("claude-opus-4-5", { exact: true }),
    ).toBeVisible();
  });

  test("profile Edit routes persona-linked agents to the definition editor", async ({
    page,
  }) => {
    // Routing pin for handleEditAgent (UserProfilePanel): when the agent has
    // a resolvable non-built-in persona, the Edit quick action opens the
    // DEFINITION editor (persona dialog), not EditAgentDialog. The instance
    // editor (and its inherit-runtime toggle) is reachable for persona-linked
    // agents only via the requestOpenEditAgent event (ConfigNudgeCard) — no
    // plain UI path — so its inherit-toggle behavior is covered by B3b's
    // component-level pinning test, not e2e.
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: AGENT_PUBKEY,
          name: AGENT_NAME,
          personaId: PERSONA_ID,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
      personas: [
        {
          id: PERSONA_ID,
          displayName: "Edit E2E Persona",
          systemPrompt: "You are the edit-agent e2e persona.",
        },
      ],
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();

    // Persona-linked agents render grouped under the persona's card name.
    const agentButton = page.getByRole("button", {
      name: "Edit E2E Persona agent profile",
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    // Merged dialog opens with D+I sections together — the old routing that
    // preferred the definition dialog over the instance dialog is fixed.
    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.getByTestId("persona-dialog")).not.toBeVisible();
    // Both D-fields (agent name from definition) and I-fields are accessible.
    await expect(page.locator("#edit-agent-display-name")).toHaveValue(
      "Edit E2E Persona",
    );
  });
});

test.describe("merged dialog — team-linked and D+I wiring", () => {
  const TEAM_PERSONA_ID = "persona-team-e2e";
  const TEAM_AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
  const TEAM_AGENT_NAME = "Tyler Agent";

  test("team-linked instance: D-fields render disabled in the merged dialog", async ({
    page,
  }) => {
    // Seed a team-managed persona so the merged dialog shows D-fields as read-only.
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: TEAM_AGENT_PUBKEY,
          name: TEAM_AGENT_NAME,
          personaId: TEAM_PERSONA_ID,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
      personas: [
        {
          id: TEAM_PERSONA_ID,
          displayName: "Team Bot",
          systemPrompt: "Team-managed agent.",
          sourceTeam: "team-acme",
        },
      ],
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();

    const agentButton = page.getByRole("button", {
      name: "Team Bot agent profile",
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });

    // D-fields must be disabled for team-managed definitions.
    await expect(page.locator("#edit-agent-display-name")).toBeDisabled();
    await expect(page.locator("#edit-agent-system-prompt")).toBeDisabled();

    // The "Managed by team X" notice must identify the team.
    await expect(page.getByTestId("team-managed-notice")).toBeVisible();
    await expect(page.getByTestId("team-managed-notice")).toContainText(
      "team-acme",
    );

    // I-fields (instance name, respond-to) remain editable.
    await expect(page.locator("#edit-agent-name")).not.toBeDisabled();
    // The access control is the regression pin for the defReadOnly fix: it is an
    // I-owned field, so it must stay enabled even though the definition is
    // team-managed and its D-fields are read-only.
    await expect(page.locator("#agent-respond-to")).not.toBeDisabled();
  });

  test("linked agent: edit D-field and I-field, one Save persists both layers", async ({
    page,
  }) => {
    // Seed a persona-linked agent. The merged dialog must write both the
    // D-layer (persona displayName) and I-layer (instance name) in a single Save.
    const LINKED_PERSONA_ID = "persona-linked-e2e";
    const LINKED_PUBKEY = TEST_IDENTITIES.tyler.pubkey;

    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey: LINKED_PUBKEY,
          name: "Original Instance Name",
          personaId: LINKED_PERSONA_ID,
          status: "stopped",
          channelNames: ["agents"],
        },
      ],
      personas: [
        {
          id: LINKED_PERSONA_ID,
          displayName: "Original Definition Name",
          systemPrompt: "Original prompt.",
        },
      ],
    });

    await page.goto("/");
    await page.getByTestId("open-agents-view").click();

    const agentButton = page.getByRole("button", {
      name: "Original Definition Name agent profile",
    });
    await expect(agentButton).toBeVisible({ timeout: 10_000 });
    await agentButton.click();

    await expect(page.getByTestId("user-profile-panel")).toBeVisible({
      timeout: 10_000,
    });
    await page.getByTestId("user-profile-edit-agent").click();

    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });

    // Edit a D-field (definition display name) and an I-field (instance name)
    // in the same dialog open.
    await page.locator("#edit-agent-display-name").fill("Renamed Definition");
    await page.locator("#edit-agent-name").fill("Renamed Instance");

    // One Save click — both layers must be persisted.
    await page.getByTestId("edit-agent-dialog-submit").click();
    await expect(page.getByTestId("edit-agent-dialog")).not.toBeVisible();

    // Reopen and verify both changes persisted (mock bridge echoes saves to store).
    await page.getByTestId("user-profile-edit-agent").click();
    await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
      timeout: 10_000,
    });
    await expect(page.locator("#edit-agent-display-name")).toHaveValue(
      "Renamed Definition",
      { timeout: 10_000 },
    );
    await expect(page.locator("#edit-agent-name")).toHaveValue(
      "Renamed Instance",
      { timeout: 10_000 },
    );
  });
});
