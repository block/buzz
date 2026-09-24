import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const SHOTS = "test-results/harness-profile-variants";

/**
 * Profile-variant harness definitions.
 *
 * A custom definition may declare a `variants` block (a profile directory plus a
 * marker file); the Rust catalog then materializes one entry per detected
 * profile. Two UI consequences follow from the definition, and both are pinned
 * here because a component could regress them without any Rust test noticing:
 *
 *   1. generated entries are read-only in Settings -> Agents -> Harnesses and
 *      name the definition file to edit instead (`harnessGalleryLogic`);
 *   2. an entry whose definition declares `model_selection: "harness"` owns its
 *      own model, so the agent dialog omits the model picker (`agentConfigCore`
 *      reason `ownedByHarnessSelection`).
 */

type RawEntry = Record<string, unknown>;

/** Shape shared by every available catalog entry in this fixture. */
function availableEntry(overrides: RawEntry): RawEntry {
  return {
    avatar_url: "",
    availability: "available",
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url: "https://hermes-agent.nousresearch.com/docs",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    ...overrides,
  };
}

/** The hand-authored definition a user writes: one file, one variants block. */
const TEMPLATE = availableEntry({
  id: "hermes-profiles",
  label: "Hermes Agent (profiles)",
  command: "hermes-acp",
  binary_path: "/usr/local/bin/hermes-acp",
  source: "custom",
  model_selection: "harness",
  definition_variants: {
    dir: "~/.hermes/profiles",
    marker: "profile.yaml",
    labelFrom: { file: "profile.yaml", key: "ui_meta.hermes-bots.title" },
    env: { HERMES_HOME: "{dir}" },
  },
});

/** One entry per profile directory the Rust expander found. */
function generatedEntry(slug: string, label: string): RawEntry {
  return availableEntry({
    id: `hermes-profile-${slug}`,
    label,
    command: "hermes-acp",
    binary_path: "/usr/local/bin/hermes-acp",
    source: "custom",
    model_selection: "harness",
    generated: true,
    generated_from: "hermes-profiles",
  });
}

const GENERATED = [
  generatedEntry("generalist", "Hermes Agent [Generalist] (generalist)"),
  generatedEntry("default", "Hermes Agent [Developer] (default)"),
];

/** A hand-authored custom harness: no variants block, still user-editable. */
const HAND_AUTHORED = availableEntry({
  id: "my-custom",
  label: "My Custom Harness",
  command: "my-acp",
  binary_path: "/usr/local/bin/my-acp",
  source: "custom",
});

const BUZZ_AGENT = availableEntry({
  id: "buzz-agent",
  label: "Buzz Agent",
  command: "buzz-agent",
  binary_path: "/usr/local/bin/buzz-agent",
});

const CATALOG = [BUZZ_AGENT, TEMPLATE, ...GENERATED, HAND_AUTHORED];

async function openHarnessesSettings(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await openSettings(page, "agents");
  await expect(page.getByTestId("settings-harnesses")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.locator(".animate-spin").first()).not.toBeVisible({
    timeout: 5_000,
  });
}

/** Menu-based PersonaDropdownField, not a native <select>. */
async function selectDropdownOption(
  page: import("@playwright/test").Page,
  trigger: import("@playwright/test").Locator,
  optionName: string | RegExp,
) {
  await expect(trigger).toBeVisible({ timeout: 10_000 });
  await trigger.press("Enter");
  await page
    .getByRole("menuitemradio", { name: optionName })
    .click({ timeout: 5_000 });
}

test("profile variants: generated entries are read-only and name their definition", async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1280, height: 1400 });
  await installMockBridge(page, { acpRuntimesCatalog: CATALOG });
  await openHarnessesSettings(page);

  // Both expanded profiles are listed as their own rows.
  for (const entry of GENERATED) {
    await expect(page.getByTestId(`doctor-runtime-${entry.id}`)).toBeVisible();
  }

  // Each generated row points at the definition file that owns it...
  const generatedNote = page.getByTestId(
    "doctor-runtime-generated-hermes-profile-generalist",
  );
  await expect(generatedNote).toBeVisible();
  await expect(generatedNote).toContainText("hermes-profiles");

  await page.getByTestId("settings-harnesses").screenshot({
    path: `${SHOTS}/harnesses-generated-entries.png`,
  });

  // Row actions live behind the row's own menu, and for an available harness the
  // menu's only remaining entries would be Edit/Delete. So the menu's absence is
  // the read-only proof: the generated row renders with a live status and no way
  // to edit or delete it, because the definition file owns both.
  await expect(
    page.getByTestId("doctor-runtime-menu-hermes-profile-generalist"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("doctor-runtime-menu-hermes-profile-default"),
  ).toHaveCount(0);

  // The definition that owns them stays editable, so the generated family can
  // be retargeted without hand-editing JSON.
  await page.getByTestId("doctor-runtime-menu-hermes-profiles").click();
  await expect(
    page.getByTestId("custom-harness-edit-hermes-profiles"),
  ).toBeVisible();
  await expect(
    page.getByTestId("custom-harness-delete-hermes-profiles"),
  ).toBeVisible();
  await page.keyboard.press("Escape");

  // A hand-authored custom harness is untouched by the expansion path.
  await page.getByTestId("doctor-runtime-menu-my-custom").click();
  await expect(page.getByTestId("custom-harness-edit-my-custom")).toBeVisible();
  await page.keyboard.press("Escape");
});

test("profile variants: a harness-owned model omits the agent model picker", async ({
  page,
}) => {
  test.setTimeout(60_000);
  await page.setViewportSize({ width: 1280, height: 1400 });
  await installMockBridge(page, { acpRuntimesCatalog: CATALOG });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();

  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await page.locator("#persona-display-name").fill("Profile Agent");
  await page.getByRole("tab", { name: "Customize for this agent" }).click();

  // Pick an expanded profile entry.
  await selectDropdownOption(
    page,
    page.locator("#persona-runtime"),
    "Hermes Agent [Generalist] (generalist)",
  );

  // The definition owns the model, so the picker is omitted rather than shown
  // and ignored. The AI mode switch that leads to it goes with it.
  await expect(
    page.getByTestId("agent-custom-configuration-section"),
  ).toBeVisible();
  await expect(page.locator("#persona-model")).toHaveCount(0);
  await expect(page.locator("#persona-custom-model")).toHaveCount(0);

  await dialog.screenshot({ path: `${SHOTS}/agent-no-model-picker.png` });

  // Control: a harness that does not own its model keeps the picker, so the
  // omission above is caused by the definition and not by the fixture.
  await selectDropdownOption(
    page,
    page.locator("#persona-runtime"),
    "Buzz Agent",
  );
  await expect(page.locator("#persona-model")).toBeVisible({ timeout: 5_000 });
});
