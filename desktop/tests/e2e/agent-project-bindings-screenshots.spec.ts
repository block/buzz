import { expect, test, type Locator, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/agent-project-bindings-screenshots";
const PERSONA_ID = "campaign-analyst";
const REQUIREMENT_ID = "weekly-analytics";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const PROJECT_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const PROJECT_SCOPE = {
  relayUrl: "ws://localhost:3000",
  operatorPubkey: DEFAULT_MOCK_PUBKEY,
  projectAddress: `30621:${DEFAULT_MOCK_PUBKEY}:buzz`,
};

const GOOGLE_ANALYTICS_CONNECTION = {
  id: "connection-google-analytics",
  projectScope: PROJECT_SCOPE,
  name: "Google Analytics",
  provider: "Google Analytics",
  capabilityIds: ["mcp.tool.run_report", "mcp.tool.export_report"],
  discoveredTools: ["run_report", "export_report"],
  command: "/opt/homebrew/bin/analytics-connector",
  args: ["--account", "acme"],
  envKeys: ["GOOGLE_ANALYTICS_TOKEN"],
  health: {
    status: "ready" as const,
    lastVerifiedAt: "2026-08-03T14:30:00.000Z",
    detail: null,
  },
  createdAt: "2026-08-03T14:00:00.000Z",
  updatedAt: "2026-08-03T14:30:00.000Z",
};

const WAREHOUSE_CONNECTION = {
  ...GOOGLE_ANALYTICS_CONNECTION,
  id: "connection-analytics-warehouse",
  name: "Analytics warehouse",
  provider: "Warehouse MCP",
  command: "/opt/homebrew/bin/warehouse-connector",
  args: ["--workspace", "acme"],
  envKeys: ["WAREHOUSE_TOKEN"],
};

async function capture(subject: Locator, filename: string) {
  await waitForAnimations(subject.page());
  await subject.screenshot({ path: `${SHOTS}/${filename}` });
}

async function choose(
  page: Page,
  triggerId: string,
  optionName: string | RegExp,
) {
  await page.locator(`#${triggerId}`).click();
  await page
    .getByRole("menuitemradio", { name: optionName })
    .click({ timeout: 5_000 });
}

async function openAgents(page: Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("agents-page-content")).toBeVisible({
    timeout: 10_000,
  });
}

test.describe("agent Project bindings screenshots", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("binds a template requirement and restarts a running agent", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      window.localStorage.setItem(
        "buzz-feature-overrides-v1",
        JSON.stringify({ projects: true }),
      );
    });
    await installMockBridge(page, {
      acpRuntimesCatalog: [
        {
          auth_status: { status: "logged_in" },
          availability: "available",
          avatar_url: "",
          binary_path: "/usr/local/bin/codex",
          can_auto_install: false,
          command: "codex",
          default_args: [],
          id: "codex",
          install_hint: "",
          install_instructions_url: "https://github.com/openai/codex",
          label: "Codex",
          login_hint: null,
          mcp_command: null,
          node_required: false,
          underlying_cli_path: "/usr/local/bin/codex",
        },
      ],
      projectChannelId: PROJECT_CHANNEL_ID,
      projectConnections: [GOOGLE_ANALYTICS_CONNECTION, WAREHOUSE_CONNECTION],
      personas: [
        {
          id: PERSONA_ID,
          displayName: "Campaign Analyst",
          systemPrompt:
            "Turn campaign performance into a clear weekly recommendation.",
          runtime: "codex",
          toolRequirements: [
            {
              id: REQUIREMENT_ID,
              label: "Weekly analytics",
              capability: "mcp.tool.run_report",
              required: true,
            },
          ],
        },
      ],
    });

    await openAgents(page);
    await page.getByTestId(`persona-runtime-start-${PERSONA_ID}`).click();

    const launchDialog = page.getByTestId("agent-project-launch-dialog");
    await expect(launchDialog).toBeVisible();
    await choose(page, "agent-project", "buzz");
    await choose(
      page,
      `agent-tool-binding-${REQUIREMENT_ID}`,
      "Google Analytics",
    );
    await expect(
      launchDialog.getByRole("button", { name: "Start agent" }),
    ).toBeEnabled();
    await capture(launchDialog, "01-agent-configuration.png");

    await launchDialog.getByRole("button", { name: "Start agent" }).click();
    await expect(launchDialog).not.toBeVisible();
    const secretDialog = page.getByRole("dialog", { name: "Agent created" });
    await expect(secretDialog).toBeVisible();
    await secretDialog.getByRole("button", { name: "Done" }).click();
    await expect(secretDialog).not.toBeVisible();
    const agentCard = page.getByTestId(`persona-agent-row-${PERSONA_ID}`);
    await expect(
      agentCard.locator('[data-testid^="agent-runtime-active-"]'),
    ).toBeVisible({ timeout: 10_000 });

    await page
      .getByRole("button", { name: "Campaign Analyst agent profile" })
      .click();
    const activeTestId = await agentCard
      .locator('[data-testid^="agent-runtime-active-"]')
      .getAttribute("data-testid");
    const agentPubkey = activeTestId?.replace("agent-runtime-active-", "");
    if (!agentPubkey) throw new Error("Created agent pubkey was not rendered.");
    await page.evaluate((pubkey) => {
      window.dispatchEvent(
        new CustomEvent("buzz:open-edit-agent", {
          detail: { pubkey },
        }),
      );
    }, agentPubkey);

    const editDialog = page.getByTestId("edit-agent-dialog");
    await expect(editDialog).toBeVisible();
    await expect(editDialog.locator("#edit-agent-project")).toContainText(
      "buzz",
    );
    await choose(
      page,
      `edit-agent-tool-binding-${REQUIREMENT_ID}`,
      "Analytics warehouse",
    );
    const saveAndRestart = editDialog.getByRole("button", {
      name: "Save and restart",
    });
    await expect(saveAndRestart).toBeEnabled();
    await editDialog
      .getByTestId("edit-agent-dialog-scroll-area")
      .evaluate((element) => element.scrollTo({ top: 0 }));
    await capture(editDialog, "02-running-agent-update-ready.png");

    await saveAndRestart.click();
    await expect(editDialog).not.toBeVisible();
    const success = page.getByText(
      "Campaign Analyst restarted with its changes.",
      { exact: true },
    );
    await expect(success).toBeVisible({ timeout: 10_000 });
    await capture(page.locator("body"), "03-running-agent-update-success.png");
  });
});
