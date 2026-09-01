/**
 * E2E spec for the create-agent "Run on" provider config fields.
 *
 * Pins the fix for the "Typewriter Eraser": WhereToRunSection's probe effect
 * used to depend on the whole draft, so every keystroke re-probed the
 * provider and every probe resolution reset providerConfig to schema
 * defaults — typing into a defaultless field (the k8s "Kubeconfig context")
 * looked completely dead, and the provider binary respawned in a loop.
 *
 * Covers:
 *  - typing into a defaultless provider field sticks, and the provider is
 *    probed exactly once for the selection (not once per keystroke or
 *    Advanced disclosure toggle)
 *  - the config form is gated on probe resolution (no half-rendered form),
 *    and defaults prefill exactly once when a slow probe lands
 *  - collapsing Advanced during an incomplete remote setup keeps the submit
 *    blocker visible through the Required badge
 *  - switching provider → local → provider re-probes and resets cleanly
 *
 * The stale-closure merge on probe resolution (defaults beneath in-flight
 * typing) is unreachable through this UI because the fields render only
 * after the probe resolves; it is pinned at the unit level in
 * whereToRunIntent.test.mjs (applyProbeResult).
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

type Page = import("@playwright/test").Page;

const PROVIDER = {
  id: "kubernetes",
  binaryPath: "/mock/buzz-backend-kubernetes",
};

const PROBE_RESULT = {
  ok: true,
  name: "kubernetes",
  version: "0.0.0-mock",
  config_schema: {
    type: "object",
    properties: {
      context: {
        type: "string",
        title: "Kubeconfig context",
        description: "Context from your kubeconfig.",
      },
      namespace: {
        type: "string",
        title: "Namespace",
        default: "buzz-agents-mock01",
      },
    },
    required: ["namespace"],
  },
};

async function probeInvocations(page: Page): Promise<number> {
  return page.evaluate(
    () =>
      (
        window as Window & { __BUZZ_E2E_COMMANDS__?: string[] }
      ).__BUZZ_E2E_COMMANDS__?.filter(
        (command) => command === "probe_backend_provider",
      ).length ?? 0,
  );
}

async function selectRunOnOption(
  page: Page,
  dialog: import("@playwright/test").Locator,
  optionName: string,
) {
  const trigger = dialog.locator("#agent-run-on");
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
  await trigger.press("Enter");

  const option = page.getByRole("menuitemradio", {
    exact: true,
    name: optionName,
  });
  await expect(option).toBeVisible();
  // The shared PersonaDropdownField supports keyboard selection. Using it here
  // avoids racing the menu's open animation when this test changes locations
  // repeatedly.
  await option.press("Enter");
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

async function configureRunnableLocalAgent(
  page: Page,
  dialog: import("@playwright/test").Locator,
) {
  await page.getByRole("tab", { name: "Customize for this agent" }).click();
  const provider = dialog.locator("#persona-llm-provider");
  await expect(provider).toBeVisible({ timeout: 10_000 });
  await provider.press("Enter");
  await page
    .getByRole("menuitemradio", { exact: true, name: "Anthropic" })
    .click();
  const model = dialog.locator("#persona-model");
  await model.click();
  await page
    .getByRole("button", { name: "Custom model...", exact: true })
    .click();
  await page.getByLabel("Custom model ID").fill("claude-opus-4-5");
  await page.getByLabel("Anthropic API Key").fill("sk-working-folder-e2e");
}

/** Open Advanced in the create-agent dialog and select the mocked provider. */
async function openCreateDialogOnProvider(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  const advanced = dialog.getByRole("button", {
    name: "Advanced",
    exact: true,
  });
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(dialog.locator("#agent-run-on")).toHaveCount(0);
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "true");
  const respondTo = dialog.getByTestId("agent-respond-to");
  const runOn = dialog.locator("#agent-run-on");
  await expect(respondTo).toBeVisible();
  await expect(runOn).toBeVisible();
  expect(await respondTo.evaluate((element) => element.offsetTop)).toBeLessThan(
    await runOn.evaluate((element) => element.offsetTop),
  );
  await selectRunOnOption(page, dialog, PROVIDER.id);
  return dialog;
}

test("typing into a defaultless provider field sticks and probes only once", async ({
  page,
}) => {
  await installMockBridge(page, {
    backendProviders: [PROVIDER],
    backendProviderProbeResult: PROBE_RESULT,
  });
  const dialog = await openCreateDialogOnProvider(page);

  const contextField = dialog.locator("#provider-cfg-context");
  await expect(contextField).toBeVisible({ timeout: 10_000 });
  // Defaults prefilled from the schema; context has none.
  await expect(dialog.locator("#provider-cfg-namespace")).toHaveValue(
    "buzz-agents-mock01",
  );
  await expect(contextField).toHaveValue("");

  await contextField.fill("prod-us-west");
  await expect(contextField).toHaveValue("prod-us-west");

  // One selection, one probe — keystrokes and Advanced disclosure toggles
  // must not refire executable provider discovery after it has completed.
  expect(await probeInvocations(page)).toBe(1);
  const advanced = dialog.getByRole("button", {
    name: "Advanced",
    exact: true,
  });
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(dialog.locator("#agent-run-on")).toHaveCount(0);
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "true");
  await expect(dialog.locator("#provider-cfg-context")).toHaveValue(
    "prod-us-west",
  );
  expect(await probeInvocations(page)).toBe(1);
});

test("config fields render only after a slow probe resolves, with defaults", async ({
  page,
}) => {
  // The fields are gated on the probe result (draft.probedProvider), which is
  // what makes mid-flight typing unreachable through the UI — the stale-probe
  // merge seam (applyProbeResult) is pinned at the unit level instead. This
  // spec holds the gate: no half-rendered form before the probe lands, and
  // defaults appear exactly once when it does.
  await installMockBridge(page, {
    backendProviders: [PROVIDER],
    backendProviderProbeResult: PROBE_RESULT,
    backendProviderProbeDelayMs: 1_000,
  });
  const dialog = await openCreateDialogOnProvider(page);

  // Pre-resolution: the security warning is up, the form is not.
  await expect(dialog.getByText("will receive your agent")).toBeVisible();
  await expect(dialog.locator("#provider-cfg-context")).toHaveCount(0);

  // Post-resolution: fields render with schema defaults prefilled.
  await expect(dialog.locator("#provider-cfg-context")).toBeVisible({
    timeout: 10_000,
  });
  await expect(dialog.locator("#provider-cfg-namespace")).toHaveValue(
    "buzz-agents-mock01",
  );
  expect(await probeInvocations(page)).toBe(1);
});

test("collapsed Advanced marks incomplete remote setup as required", async ({
  page,
}) => {
  await installMockBridge(page, {
    backendProviders: [PROVIDER],
    backendProviderProbeResult: PROBE_RESULT,
    backendProviderProbeDelayMs: 10_000,
  });
  const dialog = await openCreateDialogOnProvider(page);
  const advanced = dialog.getByRole("button", {
    name: "Advanced",
    exact: true,
  });
  const submit = dialog.getByTestId("persona-dialog-submit");

  await expect(submit).toBeDisabled();
  await advanced.click();
  await expect(advanced).toHaveAttribute("aria-expanded", "false");
  await expect(dialog.locator("#agent-run-on")).toHaveCount(0);
  await expect(
    dialog.getByTestId("persona-advanced-required-badge"),
  ).toHaveText("Required");
  await expect(submit).toBeDisabled();
});

test("local create picks a working folder and includes it only in the instance request", async ({
  page,
}) => {
  const workingDirectory = "/Users/dev/projects/alpha";
  await installMockBridge(page, {
    agentWorkingDirectoryPick: workingDirectory,
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  const dialog = page.getByTestId("persona-dialog");
  await dialog.locator("#persona-display-name").fill("Working folder agent");
  await configureRunnableLocalAgent(page, dialog);
  await dialog.getByRole("button", { name: "Advanced", exact: true }).click();

  const folder = dialog.getByTestId("agent-working-folder-field");
  await expect(folder).toBeVisible();
  await folder.getByRole("button", { name: "Choose working folder" }).click();
  await expect(folder.getByTestId("agent-working-folder-path")).toHaveText(
    workingDirectory,
  );
  await dialog.getByTestId("persona-dialog-submit").click();

  const createPayload = await page.evaluate(() => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: { input?: Record<string, unknown> };
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log?.find((entry) => entry.command === "create_managed_agent")
      ?.payload.input;
  });
  expect(createPayload?.workingDirectory).toBe(workingDirectory);
  expect(createPayload?.backend).toEqual({ type: "local" });
});

test("provider create drops a previously selected local working folder", async ({
  page,
}) => {
  const agentName = `Provider folder agent ${Date.now()}`;
  await installMockBridge(page, {
    agentWorkingDirectoryPick: "/Users/dev/projects/alpha",
    backendProviders: [PROVIDER],
    backendProviderProbeResult: PROBE_RESULT,
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  const dialog = page.getByTestId("persona-dialog");
  await dialog.locator("#persona-display-name").fill(agentName);
  await configureRunnableLocalAgent(page, dialog);
  await dialog.getByRole("button", { name: "Advanced", exact: true }).click();
  const folder = dialog.getByTestId("agent-working-folder-field");
  await folder.getByRole("button", { name: "Choose working folder" }).click();
  await expect(folder).toBeVisible();

  await selectRunOnOption(page, dialog, PROVIDER.id);
  await expect(folder).toHaveCount(0);
  await expect(dialog.locator("#provider-cfg-namespace")).toBeVisible();
  await dialog.getByTestId("persona-dialog-submit").click();

  const createPayload = await page.evaluate((name) => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: { input?: Record<string, unknown> };
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log
      ?.filter((entry) => entry.command === "create_managed_agent")
      .find((entry) => entry.payload.input?.name === name)?.payload.input;
  }, agentName);
  expect(createPayload?.workingDirectory).toBeUndefined();
  expect(createPayload?.backend).toMatchObject({ type: "provider" });
});

test("provider → local → provider re-probes and resets the config", async ({
  page,
}) => {
  await installMockBridge(page, {
    backendProviders: [PROVIDER],
    backendProviderProbeResult: PROBE_RESULT,
  });
  const dialog = await openCreateDialogOnProvider(page);

  const contextField = dialog.locator("#provider-cfg-context");
  await expect(contextField).toBeVisible({ timeout: 10_000 });
  await contextField.fill("stale-value");

  await selectRunOnOption(page, dialog, "This computer");
  await expect(contextField).toHaveCount(0);

  await selectRunOnOption(page, dialog, PROVIDER.id);
  await expect(dialog.locator("#provider-cfg-context")).toBeVisible({
    timeout: 10_000,
  });
  // Fresh selection = fresh draft: the stale value must not leak back.
  await expect(dialog.locator("#provider-cfg-context")).toHaveValue("");
  expect(await probeInvocations(page)).toBe(2);
});
