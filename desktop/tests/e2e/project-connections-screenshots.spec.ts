import { expect, test, type Locator, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/project-connections-screenshots";
const CONNECTION_ID = "connection-google-analytics";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const PROJECT_SCOPE = {
  relayUrl: "ws://localhost:3000",
  operatorPubkey: DEFAULT_MOCK_PUBKEY,
  repoAddress: `30617:${DEFAULT_MOCK_PUBKEY}:buzz`,
};

async function capture(page: Page, subject: Locator, filename: string) {
  await waitForAnimations(page);
  await subject.screenshot({ path: `${SHOTS}/${filename}` });
}

async function openConnections(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByRole("button", { name: "Repositories", exact: true }).click();
  const project = page
    .locator(
      '[data-testid="project-card-buzz"], [data-testid="project-row-buzz"]',
    )
    .first();
  await expect(project).toBeVisible({ timeout: 10_000 });
  await project.click();
  await page.getByRole("tab", { name: "Connections", exact: true }).click();
  await expect(page.getByTestId("project-connections-panel")).toBeVisible();
}

test.describe("Project Connections screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      window.localStorage.setItem(
        "buzz-feature-overrides-v1",
        JSON.stringify({ projects: true }),
      );
    });
    await installMockBridge(page, {
      projectConnections: [
        {
          id: CONNECTION_ID,
          projectScope: PROJECT_SCOPE,
          name: "Google Analytics",
          provider: "Google Analytics",
          capabilityIds: ["mcp.tool.run_report", "mcp.tool.export_report"],
          discoveredTools: ["run_report", "export_report"],
          command: "/opt/homebrew/bin/analytics-connector",
          args: ["--account", "acme"],
          envKeys: ["GOOGLE_ANALYTICS_TOKEN"],
          health: {
            status: "ready",
            lastVerifiedAt: "2026-08-02T14:30:00.000Z",
            detail: null,
          },
          createdAt: "2026-08-02T14:00:00.000Z",
          updatedAt: "2026-08-02T14:30:00.000Z",
        },
      ],
    });
  });

  test("covers setup, verification, and removal", async ({ page }) => {
    await openConnections(page);

    const panel = page.getByTestId("project-connections-panel");
    await expect(panel).toContainText("Google Analytics");
    await expect(panel).toContainText("Tools found");
    await expect(panel).toContainText("Run Report");
    await capture(page, panel, "01-ready-connection.png");

    await panel.getByRole("button", { name: "Add connection" }).click();
    const setup = page.getByRole("dialog", { name: "Add connection" });
    await setup.getByLabel("Connection name").fill("Issue tracker");
    await setup.getByLabel("Service").fill("Linear");
    await setup
      .getByLabel("Connection command")
      .fill("/opt/homebrew/bin/linear-connector");
    await setup.getByRole("button", { name: "Technical details" }).click();
    await setup
      .getByRole("textbox", { name: "Arguments", exact: true })
      .fill("--workspace\nacme");
    await setup.getByLabel("Secret 1 name").fill("LINEAR_API_TOKEN");
    await setup.getByLabel("Secret 1 value").fill("not-shown-in-capture");
    await setup
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await capture(page, setup, "02-add-connection.png");
    await setup.getByRole("button", { name: "Cancel" }).click();

    await panel.getByRole("button", { name: "Edit Google Analytics" }).click();
    const edit = page.getByRole("dialog", { name: "Edit connection" });
    await edit
      .getByLabel("Connection command")
      .fill("/opt/homebrew/bin/analytics-connector-v2");
    await edit
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await edit.getByRole("button", { name: "Save connection" }).click();
    await expect(panel).toContainText("Not tested");

    await panel.getByRole("button", { name: "Test", exact: true }).click();
    await expect(panel).toContainText("Tools found");
    await expect(panel).toContainText("Analytics.Weekly Summary");

    await panel
      .getByRole("button", { name: "Remove Google Analytics" })
      .click();
    const confirmation = page.getByRole("alertdialog", {
      name: "Remove Google Analytics?",
    });
    await capture(page, confirmation, "03-remove-connection-confirmation.png");
    await confirmation
      .getByRole("button", { name: "Remove connection" })
      .click();
    await expect(panel).toContainText("No connections yet");
  });
});
