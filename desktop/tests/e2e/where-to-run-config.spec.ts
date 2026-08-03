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
 *    probed exactly once for the selection (not once per keystroke)
 *  - the config form is gated on probe resolution (no half-rendered form),
 *    and defaults prefill exactly once when a slow probe lands
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

/** Open the create-agent dialog and select the mocked provider in "Run on". */
async function openCreateDialogOnProvider(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: "Create agent" }).click();
  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await dialog.locator("#agent-run-on").selectOption(PROVIDER.id);
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

  await contextField.pressSequentially("prod-us-west", { delay: 20 });
  await expect(contextField).toHaveValue("prod-us-west");

  // One selection, one probe — keystrokes must not refire it.
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

  await dialog.locator("#agent-run-on").selectOption("local");
  await expect(contextField).toHaveCount(0);

  await dialog.locator("#agent-run-on").selectOption(PROVIDER.id);
  await expect(dialog.locator("#provider-cfg-context")).toBeVisible({
    timeout: 10_000,
  });
  // Fresh selection = fresh draft: the stale value must not leak back.
  await expect(dialog.locator("#provider-cfg-context")).toHaveValue("");
  expect(await probeInvocations(page)).toBe(2);
});
