import { expect, test, type Locator, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import type { ProjectConnection } from "../../src/shared/api/tauriProjectConnections";

const SHOTS = "test-results/project-connections-screenshots";
const CONNECTION_ID = "connection-google-analytics";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const PROJECT_SCOPE = {
  relayUrl: "ws://localhost:3000",
  operatorPubkey: DEFAULT_MOCK_PUBKEY,
  projectAddress: `30621:${DEFAULT_MOCK_PUBKEY}:buzz`,
};

function connectionFixture(
  overrides: Partial<ProjectConnection> = {},
): ProjectConnection {
  return {
    id: CONNECTION_ID,
    projectScope: PROJECT_SCOPE,
    name: "Google Analytics",
    provider: "Google Analytics",
    capabilityIds: [
      `mcp.tool.${CONNECTION_ID}.run_report`,
      `mcp.tool.${CONNECTION_ID}.export_report`,
    ],
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
    ...overrides,
  };
}

async function capture(page: Page, subject: Locator, filename: string) {
  await waitForAnimations(page);
  await subject.screenshot({ path: `${SHOTS}/${filename}` });
}

async function captureVisible(page: Page, subject: Locator, filename: string) {
  await waitForAnimations(page);
  const box = await subject.boundingBox();
  const viewport = page.viewportSize();
  if (!box || !viewport) {
    throw new Error("Cannot capture a subject outside the current viewport");
  }

  const x = Math.max(0, box.x);
  const y = Math.max(0, box.y);
  const right = Math.min(viewport.width, box.x + box.width);
  const bottom = Math.min(viewport.height, box.y + box.height);
  if (right <= x || bottom <= y) {
    throw new Error("Cannot capture a subject outside the current viewport");
  }

  await page.screenshot({
    path: `${SHOTS}/${filename}`,
    clip: { x, y, width: right - x, height: bottom - y },
  });
}

async function openProject(page: Page, projectSlug = "buzz") {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-projects-view").click();
  await page.getByTestId("projects-section-projects").click();
  const project = page
    .locator(
      `[data-testid="project-card-${projectSlug}"], [data-testid="project-row-${projectSlug}"]`,
    )
    .first();
  await expect(project).toBeVisible({ timeout: 10_000 });
  await project.click();
}

async function openConnections(page: Page) {
  await openProject(page);
  await page.getByRole("tab", { name: "Connections", exact: true }).click();
  await expect(page.getByTestId("project-connections-panel")).toBeVisible();
}

test.describe("Project Connections screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test.beforeEach(async ({ page }, testInfo) => {
    const zeroRepositoryProject = testInfo.title.includes("zero repositories");
    await page.addInitScript(
      ({ identityPubkey, seedZeroRepositoryProject }) => {
        window.localStorage.setItem(
          "buzz-feature-overrides-v1",
          JSON.stringify({ projects: true }),
        );
        if (seedZeroRepositoryProject) {
          window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
            {
              id: "project-empty".padEnd(64, "0"),
              kind: 30621,
              pubkey: identityPubkey,
              created_at: 1_800_000_000,
              content: "",
              tags: [
                ["d", "empty"],
                ["name", "Empty Project"],
                ["description", "A Project before its first repository."],
              ],
            },
          ];
        }
      },
      {
        identityPubkey: DEFAULT_MOCK_PUBKEY,
        seedZeroRepositoryProject: zeroRepositoryProject,
      },
    );
    const staleApproval = testInfo.title.includes("stale approval");
    const multipleRows = testInfo.title.includes("only the tested row");
    const manyTools = testInfo.title.includes("all discovered tools");
    const primaryConnection = connectionFixture(
      staleApproval
        ? {
            health: {
              status: "approval_required",
              lastVerifiedAt: null,
              detail: null,
            },
          }
        : manyTools
          ? {
              capabilityIds: Array.from(
                { length: 6 },
                (_, index) => `mcp.tool.${CONNECTION_ID}.tool_${index + 1}`,
              ),
              discoveredTools: Array.from(
                { length: 6 },
                (_, index) => `tool_${index + 1}`,
              ),
            }
          : undefined,
    );
    await installMockBridge(page, {
      projectConnectionDeleteError: testInfo.title.includes("delete failure")
        ? "Keyring unavailable."
        : undefined,
      projectConnectionSaveDelayMs: testInfo.title.includes("pending save")
        ? 500
        : undefined,
      projectConnectionTestDelayMs: multipleRows ? 500 : undefined,
      projectConnectionTestError: testInfo.title.includes("test failure")
        ? "Server exited."
        : undefined,
      projectConnections: [
        ...(zeroRepositoryProject ? [] : [primaryConnection]),
        ...(multipleRows
          ? [
              connectionFixture({
                id: "connection-linear",
                name: "Linear",
                provider: "Linear",
                capabilityIds: ["mcp.tool.connection-linear.search"],
                discoveredTools: ["search"],
                command: "/opt/homebrew/bin/linear-connector",
                envKeys: ["LINEAR_API_TOKEN"],
              }),
            ]
          : []),
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

    const repositoryPicker = page.getByTestId("project-repository-picker");
    await expect(repositoryPicker).not.toBeVisible();
    await page
      .getByRole("navigation", { name: "Project breadcrumb" })
      .getByRole("button", { name: "Buzz" })
      .click();
    await expect(
      page.getByRole("tab", { name: "Overview", exact: true }),
    ).toHaveAttribute("data-state", "active");
    await expect(repositoryPicker).toBeVisible();
    await repositoryPicker.click();
    await page.getByTestId("project-repository-relay-tools").click();
    await expect(repositoryPicker).toContainText("relay-tools");
    await page.getByRole("tab", { name: "Connections", exact: true }).click();
    await expect(page.getByTestId("project-connections-panel")).toContainText(
      "Google Analytics",
    );

    await panel.getByRole("button", { name: "Add connection" }).click();
    const setup = page.getByRole("dialog", {
      name: "Add Project connection",
    });
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
    await setup.getByRole("button", { name: "Save and test" }).click();
    const issueTrackerRow = panel
      .locator('[data-testid^="project-connection-"]')
      .filter({ hasText: "Issue tracker" });
    await expect(issueTrackerRow).toContainText("Tools found");
    await expect(issueTrackerRow).toContainText("Linear.Search Issues");

    const analyticsRow = panel.getByTestId(
      `project-connection-${CONNECTION_ID}`,
    );
    await analyticsRow
      .getByRole("button", { name: "Edit Google Analytics" })
      .click();
    const edit = page.getByRole("dialog", { name: "Google Analytics" });
    await edit
      .getByLabel("Connection command")
      .fill("/opt/homebrew/bin/analytics-connector-v2");
    await edit
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await edit.getByRole("button", { name: "Save and test" }).click();
    await expect(analyticsRow).toContainText("Tools found");
    await expect(analyticsRow).toContainText("Analytics.Weekly Summary");

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
    await expect(panel).not.toContainText("Google Analytics");
    await expect(panel).toContainText("Issue tracker");
  });

  test("stale approval can be reviewed without changing the command", async ({
    page,
  }) => {
    await openConnections(page);

    const panel = page.getByTestId("project-connections-panel");
    await expect(
      panel.getByRole("button", { name: "Review command Google Analytics" }),
    ).toBeVisible();
    await panel
      .getByRole("button", { name: "Review command Google Analytics" })
      .click();
    const dialog = page.getByRole("dialog", { name: "Google Analytics" });
    await expect(
      dialog.getByLabel(/I trust this executable and the arguments above/),
    ).toBeVisible();
    await dialog
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await dialog.getByRole("button", { name: "Save and test" }).click();

    await expect(panel).toContainText("Tools found");
    await expect(panel).toContainText("Analytics.Weekly Summary");
  });

  test("only the tested row reports progress", async ({ page }) => {
    await openConnections(page);

    const panel = page.getByTestId("project-connections-panel");
    await panel
      .getByRole("button", { name: "Test again Google Analytics" })
      .click();

    await expect(panel.getByText("Testing Google Analytics…")).toBeVisible();
    await expect(
      panel.getByRole("button", { name: "Testing Google Analytics" }),
    ).toBeDisabled();
    await expect(
      panel.getByRole("button", { name: "Test again Linear" }),
    ).toBeDisabled();
    await expect(panel.getByText("Testing Linear…")).toHaveCount(0);
    await expect(panel.getByText("Testing Google Analytics…")).toHaveCount(0, {
      timeout: 2_000,
    });
  });

  test("all discovered tools can be inspected", async ({ page }) => {
    await openConnections(page);

    const row = page.getByTestId(`project-connection-${CONNECTION_ID}`);
    await expect(row).toContainText("Tool 1");
    await expect(row).not.toContainText("Tool 6");
    await row.getByRole("button", { name: "Show 2 more" }).click();
    await expect(row).toContainText("Tool 6");
    await expect(
      row.getByRole("button", { name: "Show fewer" }),
    ).toHaveAttribute("aria-expanded", "true");
    await row.getByRole("button", { name: "Show fewer" }).click();
    await expect(row).not.toContainText("Tool 6");
  });

  test("delete failure stays in the confirmation flow", async ({ page }) => {
    await openConnections(page);

    const panel = page.getByTestId("project-connections-panel");
    await panel
      .getByRole("button", { name: "Remove Google Analytics" })
      .click();
    const confirmation = page.getByRole("alertdialog", {
      name: "Remove Google Analytics?",
    });
    await confirmation
      .getByRole("button", { name: "Remove connection" })
      .click();

    await expect(confirmation).toBeVisible();
    await expect(confirmation).toContainText(
      "Couldn't remove Google Analytics: Keyring unavailable.",
    );
    await expect(panel).toContainText("Google Analytics");
  });

  test("remains usable at maximum text zoom", async ({ page }) => {
    await page.setViewportSize({ width: 800, height: 700 });
    await openProject(page);
    await page.evaluate(() => {
      document.documentElement.style.fontSize = "24px";
      window.localStorage.setItem("buzz:text-scale", "1.5");
    });
    const connectionsTab = page.getByRole("tab", {
      name: "Connections",
      exact: true,
    });
    await expect(connectionsTab).toBeInViewport();
    await connectionsTab.click();

    const panel = page.getByTestId("project-connections-panel");
    await expect(panel).toContainText("Google Analytics");
    await expect
      .poll(() =>
        panel.evaluate((element) => element.scrollWidth <= element.clientWidth),
      )
      .toBe(true);
    await capture(page, panel, "04-maximum-text-zoom.png");
  });

  test("create flow remains operable at maximum text zoom", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 800, height: 700 });
    await openConnections(page);
    await page.evaluate(() => {
      document.documentElement.style.fontSize = "24px";
      window.localStorage.setItem("buzz:text-scale", "1.5");
    });
    await page
      .getByTestId("project-connections-panel")
      .getByRole("button", { name: "Add connection" })
      .click();

    const dialog = page.getByRole("dialog", {
      name: "Add Project connection",
    });
    await expect
      .poll(() =>
        dialog.evaluate(
          (element) => element.scrollWidth <= element.clientWidth,
        ),
      )
      .toBe(true);
    await dialog.getByLabel("Connection name").fill("Linear");
    await dialog.getByLabel("Service").fill("Linear");
    await dialog.getByLabel("Connection command").fill("/usr/bin/true");
    await dialog.getByRole("button", { name: "Technical details" }).click();
    await dialog.getByLabel("Secret 1 name").fill("LINEAR_API_TOKEN");
    await dialog.getByLabel("Secret 1 value").fill("test-only");
    await dialog
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await dialog
      .getByRole("button", { name: "Save and test" })
      .scrollIntoViewIfNeeded();
    await expect(
      dialog.getByRole("button", { name: "Save and test" }),
    ).toBeInViewport();
    await captureVisible(page, dialog, "05-maximum-text-zoom-setup.png");

    await page.keyboard.press("Tab");
    await expect
      .poll(() =>
        dialog.evaluate((element) => element.contains(document.activeElement)),
      )
      .toBe(true);
    await dialog.getByRole("button", { name: "Save and test" }).click();
    const created = page
      .locator('[data-testid^="project-connection-"]')
      .filter({ hasText: "Linear" });
    await expect(created).toContainText("Tools found");
  });

  test("pending save cannot be dismissed", async ({ page }) => {
    await openConnections(page);

    await page
      .getByTestId("project-connections-panel")
      .getByRole("button", { name: "Add connection" })
      .click();
    const dialog = page.getByRole("dialog", {
      name: "Add Project connection",
    });
    await dialog.getByLabel("Connection name").fill("Linear");
    await dialog.getByLabel("Service").fill("Linear");
    await dialog
      .getByLabel("Connection command")
      .fill("/opt/homebrew/bin/linear-connector");
    await dialog
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await dialog.getByRole("button", { name: "Save and test" }).click();

    await expect(dialog.getByRole("button", { name: "Saving…" })).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Close" })).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveCount(0, { timeout: 2_000 });
  });

  test("matches backend validation boundaries before saving", async ({
    page,
  }) => {
    await openConnections(page);

    await page
      .getByTestId("project-connections-panel")
      .getByRole("button", { name: "Add connection" })
      .click();
    const dialog = page.getByRole("dialog", {
      name: "Add Project connection",
    });
    const submit = dialog.getByRole("button", { name: "Save and test" });
    await expect(submit).toBeDisabled();

    await dialog.getByLabel("Connection name").fill("x".repeat(128));
    await expect(dialog).not.toContainText(
      "Keep the connection name to 128 bytes or fewer.",
    );
    await dialog.getByLabel("Connection name").fill("😀".repeat(33));
    await expect(dialog).toContainText(
      "Keep the connection name to 128 bytes or fewer.",
    );
    await dialog.getByLabel("Connection name").fill("Analytics");

    await dialog.getByLabel("Service").fill("x".repeat(64));
    await expect(dialog).not.toContainText(
      "Keep the service name to 64 bytes or fewer.",
    );
    await dialog.getByLabel("Service").fill("é".repeat(33));
    await expect(dialog).toContainText(
      "Keep the service name to 64 bytes or fewer.",
    );
    await dialog.getByLabel("Service").fill("Google Analytics");

    await dialog.getByLabel("Connection command").fill(`/${"x".repeat(1_023)}`);
    await expect(dialog).not.toContainText(
      "Keep the command to 1024 bytes or fewer.",
    );
    await dialog.getByLabel("Connection command").fill(`/${"😀".repeat(256)}`);
    await expect(dialog).toContainText(
      "Keep the command to 1024 bytes or fewer.",
    );
    await dialog.getByLabel("Connection command").fill("/usr/bin/true");

    await dialog.getByRole("button", { name: "Technical details" }).click();
    await dialog.getByLabel("Secret 1 name").fill("BUZZ_PRIVATE_KEY");
    await dialog.getByLabel("Secret 1 value").fill("not-allowed");
    await expect(dialog).toContainText(
      "BUZZ_PRIVATE_KEY is managed by Buzz and cannot be used here.",
    );
    await expect(submit).toBeDisabled();
    await dialog.getByLabel("Secret 1 name").fill("");
    await dialog.getByLabel("Secret 1 value").fill("");

    const argumentsInput = dialog.getByRole("textbox", {
      name: "Arguments",
      exact: true,
    });
    await argumentsInput.fill("x".repeat(4_096));
    await expect(dialog).not.toContainText(
      "Use no more than 128 arguments, with each 4096 bytes or fewer.",
    );
    await argumentsInput.fill("😀".repeat(1_025));
    await expect(dialog).toContainText(
      "Use no more than 128 arguments, with each 4096 bytes or fewer.",
    );
    await argumentsInput.fill("--version");

    await expect(submit).toBeDisabled();
    await dialog
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await expect(submit).toBeEnabled();
  });

  test("test failure leaves a recoverable row state", async ({ page }) => {
    await openConnections(page);

    const panel = page.getByTestId("project-connections-panel");
    await panel
      .getByRole("button", { name: "Test again Google Analytics" })
      .click();

    await expect(panel).toContainText("Unavailable");
    await expect(panel).toContainText(
      "The MCP server did not respond in time.",
    );
    await expect(
      panel.getByRole("button", { name: "Test again Google Analytics" }),
    ).toBeEnabled();
  });

  test("a Project with zero repositories can create and test a connection", async ({
    page,
  }) => {
    await openProject(page, "empty");

    const panel = page.getByTestId("project-connections-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText("No connections yet");
    await panel.getByRole("button", { name: "Add connection" }).first().click();
    const dialog = page.getByRole("dialog", {
      name: "Add Project connection",
    });
    await dialog.getByLabel("Connection name").fill("Linear");
    await dialog.getByLabel("Service").fill("Linear");
    await dialog
      .getByLabel("Connection command")
      .fill("/opt/homebrew/bin/linear-connector");
    await dialog
      .getByLabel(/I trust this executable and the arguments above/)
      .check();
    await dialog.getByRole("button", { name: "Save and test" }).click();

    await expect(panel).toContainText("Linear");
    await expect(panel).toContainText("Tools found");
  });
});
