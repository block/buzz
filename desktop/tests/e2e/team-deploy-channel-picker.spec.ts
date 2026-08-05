import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const OUTDIR = "test-results/team-deploy-channel-picker";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    personas: [
      {
        id: "custom:deploy-picker",
        displayName: "Deploy picker agent",
        systemPrompt: "A test agent for the deploy channel picker.",
      },
    ],
    teams: [
      {
        id: "deploy-picker-team",
        name: "Deploy picker team",
        personaIds: ["custom:deploy-picker"],
      },
    ],
  });
});

async function waitForInvokeBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => {
      const tauriWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        __TAURI_INTERNALS__?: {
          invoke?: unknown;
        };
      };

      return (
        typeof tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
        typeof tauriWindow.__TAURI_INTERNALS__?.invoke === "function"
      );
    },
    undefined,
    { timeout: 15_000 },
  );
}

async function openDeployDialog(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: "Deploy picker team team actions" })
    .click();
  await page.getByRole("menuitem", { name: "Deploy to channel" }).click();
  const dialog = page.getByRole("dialog", { name: "Deploy team to channel" });
  await expect(dialog).toBeVisible();
  return dialog;
}

test("channel picker lists joined channels first", async ({ page }) => {
  const dialog = await openDeployDialog(page);

  // Scope to the picker's listbox — the dialog's native role <select> also
  // exposes option elements.
  const options = dialog
    .getByRole("listbox", { name: "Channels" })
    .getByRole("option");
  const names = await options.allInnerTexts();

  // The mock identity is a member of `engineering` but not `design`, so
  // members-first ordering must beat plain alphabetical order.
  const engineeringIndex = names.findIndex((name) =>
    name.includes("engineering"),
  );
  const designIndex = names.findIndex((name) => name.includes("design"));
  expect(engineeringIndex).toBeGreaterThanOrEqual(0);
  expect(designIndex).toBeGreaterThanOrEqual(0);
  expect(engineeringIndex).toBeLessThan(designIndex);

  // The first (joined) channel is auto-selected.
  await expect(options.first()).toHaveAttribute("aria-selected", "true");

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/01-picker-default.png` });
});

test("search filters the channel list and moves the selection", async ({
  page,
}) => {
  const dialog = await openDeployDialog(page);
  const searchInput = dialog.locator("#team-channel-id");
  const options = dialog
    .getByRole("listbox", { name: "Channels" })
    .getByRole("option");

  await searchInput.fill("eng");

  // `engineering` matches on name (best score); `design` matches "eng" only
  // via its description, so it ranks second. Everything else drops out.
  await expect(options).toHaveCount(2);
  await expect(options.first()).toContainText("engineering");
  await expect(options.nth(1)).toContainText("design");
  // Filtering moved the selection onto the remaining match.
  await expect(options.first()).toHaveAttribute("aria-selected", "true");

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/02-picker-filtered.png` });

  await searchInput.fill("no-such-channel");
  await expect(dialog.getByText(/No channels match/)).toBeVisible();

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/03-picker-no-match.png` });
});
