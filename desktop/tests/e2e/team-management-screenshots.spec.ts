import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/team-management";

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
    null,
    { timeout: 5_000 },
  );
}

async function invokeMockCommand(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<unknown> {
  await waitForInvokeBridge(page);

  return page.evaluate(
    async ({ command: cmd, payload: pl }) => {
      const tauriWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
        __TAURI_INTERNALS__?: {
          invoke?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<unknown>;
        };
      };

      const invoke =
        tauriWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ??
        tauriWindow.__TAURI_INTERNALS__?.invoke;
      if (!invoke) {
        throw new Error("Mock invoke bridge is unavailable.");
      }

      return invoke(cmd, pl);
    },
    { command, payload },
  );
}

async function activatePersonas(page: import("@playwright/test").Page) {
  for (const id of ["builtin:fizz"]) {
    await invokeMockCommand(page, "set_persona_active", { id, active: true });
  }
}

async function openAgentsView(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);
  await activatePersonas(page);
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-library-teams")).toBeVisible({
    timeout: 10_000,
  });
}

test.describe("team management screenshots", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("00 — agents section with cards", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const agentsSection = page.getByTestId("agents-library-personas");
    await expect(agentsSection).toContainText("Architect");
    await expect(agentsSection).toContainText("Sketch");

    await agentsSection.screenshot({
      path: `${SHOTS}/00-agents-section.png`,
    });
  });

  test("01 — teams section with cards", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const teamsSection = page.getByTestId("agents-library-teams");
    await expect(teamsSection).toContainText("Engineering");
    await expect(teamsSection).toContainText("Research Agents");
    await expect(teamsSection).toContainText("Platform Tools");

    await teamsSection.screenshot({
      path: `${SHOTS}/01-teams-section.png`,
    });
  });

  test("02 — regular team context menu", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const teamsSection = page.getByTestId("agents-library-teams");
    const engineeringCard = teamsSection
      .locator("[class*='card']")
      .filter({ hasText: "Engineering" })
      .first();
    await engineeringCard.locator("button").last().click();

    const deployItem = page.getByRole("menuitem", {
      name: "Deploy to channel",
    });
    await expect(deployItem).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Edit" })).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: "Duplicate" }),
    ).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Export" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Delete" })).toBeVisible();

    await deployItem.evaluate((el) =>
      Promise.all(
        el
          .closest("[data-state]")
          ?.getAnimations()
          .map((a) => a.finished) ?? [],
      ),
    );

    await page.screenshot({
      path: `${SHOTS}/02-team-card-menu.png`,
      clip: { x: 0, y: 0, width: 1280, height: 720 },
    });
  });

  test("03 — directory-backed team with version badge", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const teamsSection = page.getByTestId("agents-library-teams");
    const versionBadge = teamsSection.locator("span", { hasText: "v1.2.0" });
    await expect(versionBadge).toBeVisible();

    const researchCard = teamsSection
      .locator("[class*='card']")
      .filter({ hasText: "Research Agents" })
      .first();
    await expect(researchCard).toBeVisible();

    await researchCard.screenshot({
      path: `${SHOTS}/03-directory-team-card.png`,
    });
  });

  test("04 — directory team context menu", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const teamsSection = page.getByTestId("agents-library-teams");
    const researchCard = teamsSection
      .locator("[class*='card']")
      .filter({ hasText: "Research Agents" })
      .first();
    await researchCard.locator("button").last().click();

    const syncItem = page.getByRole("menuitem", {
      name: "Sync from directory",
    });
    await expect(syncItem).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: "Reveal in Finder" }),
    ).toBeVisible();

    await syncItem.evaluate((el) =>
      Promise.all(
        el
          .closest("[data-state]")
          ?.getAnimations()
          .map((a) => a.finished) ?? [],
      ),
    );

    await page.screenshot({
      path: `${SHOTS}/04-directory-team-menu.png`,
      clip: { x: 0, y: 0, width: 1280, height: 720 },
    });
  });

  test("05 — symlinked team with link icon", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const teamsSection = page.getByTestId("agents-library-teams");
    const platformCard = teamsSection
      .locator("[class*='card']")
      .filter({ hasText: "Platform Tools" })
      .first();
    await expect(platformCard).toBeVisible();

    const versionBadge = platformCard.locator("span", { hasText: "v2.0.1" });
    await expect(versionBadge).toBeVisible();

    await platformCard.screenshot({
      path: `${SHOTS}/05-symlinked-team.png`,
    });
  });

  test("06 — install from directory in create modal", async ({ page }) => {
    await installMockBridge(page);
    await openAgentsView(page);

    const teamsSection = page.getByTestId("agents-library-teams");
    await expect(
      teamsSection.getByRole("button", { name: "Install from directory" }),
    ).toHaveCount(0);
    await expect(
      teamsSection.getByRole("button", { name: "Import" }),
    ).toHaveCount(0);

    await teamsSection.getByRole("button", { name: "Create team" }).click();
    const createDialog = page.getByRole("dialog", { name: "Create team" });
    const installButton = createDialog.getByRole("button", {
      name: "Install from directory",
    });
    await expect(installButton).toBeVisible();

    await createDialog.screenshot({
      path: `${SHOTS}/06-install-from-directory-modal.png`,
    });
  });
});
