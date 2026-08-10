import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const OUTDIR = "test-results/team-deploy-channel-picker";
const AGENT_PERSONA_ID = "custom:deploy-picker";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    personas: [
      {
        id: AGENT_PERSONA_ID,
        displayName: "Deploy picker agent",
        systemPrompt: "A test agent for the deploy channel picker.",
      },
    ],
    createManagedAgentDelayMs: 1_000,
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "Deploy picker agent",
        personaId: AGENT_PERSONA_ID,
        status: "running",
      },
    ],
    teams: [
      {
        id: "deploy-picker-team",
        name: "Deploy picker team",
        personaIds: [AGENT_PERSONA_ID],
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

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("open-agents-view").click();
}

async function openTeamDeployDialog(page: import("@playwright/test").Page) {
  await openAgentsView(page);
  await page
    .getByRole("button", { name: "Deploy picker team team actions" })
    .click();
  await page.getByRole("menuitem", { name: "Deploy to channel" }).click();
  const dialog = page.getByRole("dialog", { name: "Deploy team to channel" });
  await expect(dialog).toBeVisible();
  return dialog;
}

async function openAgentAddDialog(page: import("@playwright/test").Page) {
  await openAgentsView(page);
  await page.getByTestId(`persona-agent-row-${AGENT_PERSONA_ID}`).click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
  await page.getByTestId("user-profile-tab-channels").click();
  await page.getByTestId("user-profile-agent-add-channel").click();
  const dialog = page.getByRole("dialog", { name: "Add agent to channel" });
  await expect(dialog).toBeVisible();
  return dialog;
}

test("channel picker lists joined channels first and exposes the selection to AT", async ({
  page,
}) => {
  const dialog = await openTeamDeployDialog(page);

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

  // Combobox semantics: the search input announces the keyboard-selected
  // option via aria-activedescendant, and arrow keys move it. Scope by
  // accessible name — the native Role <select> is also a combobox.
  const searchInput = dialog.getByRole("combobox", { name: "Channel" });
  const firstOptionId = await options.first().getAttribute("id");
  const secondOptionId = await options.nth(1).getAttribute("id");
  expect(firstOptionId).toBeTruthy();
  await expect(searchInput).toHaveAttribute(
    "aria-activedescendant",
    firstOptionId ?? "",
  );

  await searchInput.press("ArrowDown");
  await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(searchInput).toHaveAttribute(
    "aria-activedescendant",
    secondOptionId ?? "",
  );
  await searchInput.press("ArrowUp");
  await expect(searchInput).toHaveAttribute(
    "aria-activedescendant",
    firstOptionId ?? "",
  );

  // Mouse selection must preserve focus on the combobox so keyboard
  // navigation can continue without an extra click back into the input.
  await options.nth(1).click();
  await expect(searchInput).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(options.first()).toHaveAttribute("aria-selected", "true");

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/01-picker-default.png` });
});

test("search filters the channel list and a no-match query blocks deploy", async ({
  page,
}) => {
  const dialog = await openTeamDeployDialog(page);
  const searchInput = dialog.locator("#team-channel-id");
  const options = dialog
    .getByRole("listbox", { name: "Channels" })
    .getByRole("option");
  const deployButton = dialog.getByRole("button", { name: /^Deploy \d/ });

  await expect(deployButton).toBeEnabled();

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

  // Change to a no-match query and immediately attempt submission in the same
  // browser task, without waiting for the empty-state render. The input event
  // must synchronously clear the selected channel so the native click is a
  // no-op rather than deploying to the previously visible target.
  await dialog.evaluate((root) => {
    const input = root.querySelector<HTMLInputElement>("#team-channel-id");
    const button = Array.from(root.querySelectorAll("button")).find(
      (candidate) => candidate.textContent?.startsWith("Deploy "),
    );
    const valueSetter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    if (!(input instanceof HTMLInputElement) || !button || !valueSetter) {
      throw new Error("Channel picker controls are unavailable");
    }

    valueSetter.call(input, "no-such-channel");
    input.dispatchEvent(
      new InputEvent("input", {
        bubbles: true,
        data: "no-such-channel",
        inputType: "insertText",
      }),
    );
    button.click();
  });

  await expect(dialog.getByText(/No channels match/)).toBeVisible();
  await expect(searchInput).not.toHaveAttribute("aria-activedescendant", /.+/);
  await expect(deployButton).toBeDisabled();
  await expect(deployButton).toHaveText("Deploy 1 agent");

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/03-picker-no-match.png` });

  // Clearing the query restores the default (first joined) selection and
  // re-enables deploy.
  await searchInput.fill("");
  await expect(options.first()).toHaveAttribute("aria-selected", "true");
  await expect(deployButton).toBeEnabled();
});

test("agent add-to-channel dialog gets the same picker and no-match guard", async ({
  page,
}) => {
  const dialog = await openAgentAddDialog(page);
  const searchInput = dialog.locator("#agent-channel-id");
  const options = dialog
    .getByRole("listbox", { name: "Channels" })
    .getByRole("option");
  const submitButton = dialog.getByRole("button", {
    name: /add to channel/i,
  });

  // A joined channel is auto-selected first; submit is available.
  const names = await options.allInnerTexts();
  const engineeringIndex = names.findIndex((name) =>
    name.includes("engineering"),
  );
  const designIndex = names.findIndex((name) => name.includes("design"));
  expect(engineeringIndex).toBeGreaterThanOrEqual(0);
  expect(engineeringIndex).toBeLessThan(designIndex);
  await expect(options.first()).toContainText("joined");
  await expect(options.first()).toHaveAttribute("aria-selected", "true");
  await expect(submitButton).toBeEnabled();

  await searchInput.fill("eng");
  await expect(options).toHaveCount(2);
  await expect(options.first()).toContainText("engineering");

  await searchInput.fill("no-such-channel");
  await expect(dialog.getByText(/No channels match/)).toBeVisible();
  await expect(submitButton).toBeDisabled();

  await waitForAnimations(page);
  await dialog.screenshot({ path: `${OUTDIR}/04-agent-dialog-no-match.png` });
});
