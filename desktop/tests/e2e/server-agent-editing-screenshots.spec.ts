import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

/**
 * Screenshots for editing an agent that lives on a server.
 *
 * A relocated agent is configured on the spawner, not on this Mac, so the Edit
 * dialog drops the local harness picker and scopes provider/model to whatever
 * the spawner advertised in its kind:10180 announcement. Both branches are
 * captured: a spawner that published an `ai` catalog, and one that did not.
 *
 * The "Update pending" chip is deliberately not covered here. It reads the
 * prompt-update queue, which only hydrates from localStorage once the relay
 * origin resolves — and that resolution is a one-shot, 5 s eager poll at module
 * load (`mediaUrl.ts`), so under parallel workers it can lapse before the mock
 * IPC bridge is answering and never retry. Seeding the queue key therefore
 * produces a flaky shot; the chip stays component-test territory.
 */

// Spawner that advertises an AI catalog, and one that stays silent about it.
const SPAWNER_WITH_CATALOG = "5c".repeat(32);
const SPAWNER_WITHOUT_CATALOG = "7a".repeat(32);

const CATALOG_AGENT_PUBKEY = "a1".repeat(32);
const CATALOG_AGENT_NAME = "Prod Runner";
const BARE_AGENT_PUBKEY = "b2".repeat(32);
const BARE_AGENT_NAME = "Edge Runner";

const SCREENSHOT_DIR = "test-results/server-agent-editing";

const SPAWNER_ANNOUNCEMENTS = [
  {
    pubkey: SPAWNER_WITH_CATALOG,
    content: {
      name: "prod-vps",
      runtime: "claude-agent-acp",
      max_agents: 4,
      agents_running: 1,
      ai: [
        {
          id: "anthropic",
          models: ["claude-opus-5", "claude-sonnet-5", "claude-haiku-4-5"],
        },
      ],
    },
  },
  {
    pubkey: SPAWNER_WITHOUT_CATALOG,
    content: {
      name: "edge-box",
      runtime: "claude-agent-acp",
      max_agents: 2,
      agents_running: 1,
    },
  },
];

const MANAGED_AGENTS = [
  {
    pubkey: CATALOG_AGENT_PUBKEY,
    name: CATALOG_AGENT_NAME,
    status: "stopped" as const,
    relocatedToSpawner: SPAWNER_WITH_CATALOG,
  },
  {
    pubkey: BARE_AGENT_PUBKEY,
    name: BARE_AGENT_NAME,
    status: "stopped" as const,
    relocatedToSpawner: SPAWNER_WITHOUT_CATALOG,
  },
];

// A server agent held stopped because its owner has not provisioned a token.
const HELD_AGENT_SLUG = "prod-runner";

const SPAWNER_STATUSES = [
  {
    spawnerPubkey: SPAWNER_WITH_CATALOG,
    slug: HELD_AGENT_SLUG,
    content: {
      phase: "stopped",
      agent_pubkey: CATALOG_AGENT_PUBKEY,
      needs_credential: true,
    },
  },
];

async function install(page: import("@playwright/test").Page) {
  // Seeded before the bridge installs: the section reads the store on mount and
  // the bridge is what triggers mount.
  await page.addInitScript(
    (spawners) => {
      window.localStorage.setItem(
        "buzz:spawner-pubkeys",
        JSON.stringify(spawners),
      );
    },
    [SPAWNER_WITH_CATALOG, SPAWNER_WITHOUT_CATALOG],
  );
  await installMockBridge(page, {
    managedAgents: MANAGED_AGENTS,
    spawnerAnnouncements: SPAWNER_ANNOUNCEMENTS,
    spawnerStatuses: SPAWNER_STATUSES,
  });
}

async function gotoAgents(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(() => {
    const w = window as Window & {
      __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
      __TAURI_INTERNALS__?: { invoke?: unknown };
    };
    return (
      typeof w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
      typeof w.__TAURI_INTERNALS__?.invoke === "function"
    );
  });
  await page.getByTestId("open-agents-view").click();
  await expect(
    page.getByTestId(`managed-agent-${CATALOG_AGENT_PUBKEY}`),
  ).toBeVisible({ timeout: 15_000 });
}

/** Open the Edit dialog through the profile panel — its only mount path. */
async function openEditDialog(
  page: import("@playwright/test").Page,
  agentName: string,
) {
  await page
    .getByRole("button", { name: `${agentName} agent profile` })
    .click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
    timeout: 10_000,
  });
  // The banner is the server branch's first field — its presence means the
  // spawner directory resolved and the form settled.
  await expect(page.getByTestId("server-runs-on-banner")).toBeVisible({
    timeout: 10_000,
  });
}

test.describe("server-aware agent editing", () => {
  test("shows a relocated agent on the agents screen", async ({ page }) => {
    await install(page);
    await gotoAgents(page);

    const row = page.getByTestId(`managed-agent-${CATALOG_AGENT_PUBKEY}`);
    // A relocated agent can't be started from this Mac, so the start button is
    // replaced by a server badge.
    await expect(
      page.getByTestId(`agent-runtime-start-${CATALOG_AGENT_PUBKEY}-relocated`),
    ).toBeVisible();

    await waitForAnimations(page);
    await row.screenshot({ path: `${SCREENSHOT_DIR}/01-relocated-row.png` });
  });

  test("scopes the model picker to the spawner's catalog", async ({ page }) => {
    await install(page);
    await gotoAgents(page);
    await openEditDialog(page, CATALOG_AGENT_NAME);

    const dialog = page.getByTestId("edit-agent-dialog");
    await expect(dialog).toContainText("Runs on prod-vps");
    await expect(dialog).toContainText(
      "Applied on the server. Saving restarts the agent.",
    );
    // The local harness picker is gone: the spawner decides what it runs.
    await expect(dialog.locator("#edit-agent-runtime")).toHaveCount(0);

    await page.locator("#edit-agent-model").click();
    const menu = page.getByRole("menu");
    await expect(menu).toBeVisible();
    await expect(
      menu.getByRole("menuitemradio", { name: "claude-opus-5" }),
    ).toBeVisible();

    await waitForAnimations(page);
    await page.screenshot({
      path: `${SCREENSHOT_DIR}/02-server-model-catalog.png`,
      fullPage: false,
    });
  });

  test("falls back to free text when the spawner has no catalog", async ({
    page,
  }) => {
    await install(page);
    await gotoAgents(page);
    await openEditDialog(page, BARE_AGENT_NAME);

    const dialog = page.getByTestId("edit-agent-dialog");
    await expect(dialog).toContainText("Runs on edge-box");
    await expect(dialog).toContainText(
      "Model list unavailable from this server",
    );
    await expect(dialog.locator("#edit-agent-model")).toHaveAttribute(
      "placeholder",
      "Model ID",
    );

    await waitForAnimations(page);
    await dialog.screenshot({
      path: `${SCREENSHOT_DIR}/03-no-catalog-fallback.png`,
    });
  });

  test("offers a write-only credential card per connected spawner", async ({
    page,
  }) => {
    await install(page);
    await gotoAgents(page);

    const card = page.getByTestId("spawner-credential-card").first();
    await expect(card).toBeVisible({ timeout: 15_000 });
    await expect(card).toContainText("Your Claude credential");
    await expect(card).toContainText("never stored on this device");
    // Password-type input: the token is never rendered back.
    await expect(card.getByTestId("spawner-credential-input")).toHaveAttribute(
      "type",
      "password",
    );

    await waitForAnimations(page);
    await card.screenshot({ path: `${SCREENSHOT_DIR}/04-credential-card.png` });
  });

  test("flags an agent held for a missing owner credential", async ({
    page,
  }) => {
    await install(page);
    await gotoAgents(page);

    const row = page
      .locator("li")
      .filter({ hasText: "Needs credential" })
      .first();
    await expect(row).toBeVisible({ timeout: 15_000 });
    await expect(row).toContainText(HELD_AGENT_SLUG);
    await expect(row).toContainText(
      "Add your Claude credential below to start this agent.",
    );

    await waitForAnimations(page);
    await row.screenshot({
      path: `${SCREENSHOT_DIR}/05-needs-credential-badge.png`,
    });
  });
});
