import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

// ── helpers ─────────────────────────────────────────────────────────────────

async function gotoApp(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await waitForInvokeBridge(page);
  await expect(page.getByTestId("open-agents-view")).toBeVisible({
    timeout: 10_000,
  });
}

async function waitForInvokeBridge(page: import("@playwright/test").Page) {
  await page.waitForFunction(
    () => {
      const w = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        __TAURI_INTERNALS__?: { invoke?: unknown };
      };
      return (
        typeof w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function" ||
        typeof w.__TAURI_INTERNALS__?.invoke === "function"
      );
    },
    null,
    { timeout: 5_000 },
  );
}

async function invokeTauri<T>(
  page: import("@playwright/test").Page,
  command: string,
  payload?: Record<string, unknown>,
): Promise<T> {
  await waitForInvokeBridge(page);
  return page.evaluate(
    async ({ command: c, payload: p }) => {
      const w = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          c: string,
          p?: Record<string, unknown>,
        ) => Promise<unknown>;
        __TAURI_INTERNALS__?: {
          invoke?: (c: string, p?: Record<string, unknown>) => Promise<unknown>;
        };
      };
      const invoke =
        w.__BUZZ_E2E_INVOKE_MOCK_COMMAND__ ?? w.__TAURI_INTERNALS__?.invoke;
      if (!invoke) throw new Error("Mock invoke bridge is unavailable.");
      return (await invoke(c, p)) as T;
    },
    { command, payload },
  );
}

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const AGENT_NAME = "MCP Test Agent";

async function openEditDialog(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  const agentButton = page.getByRole("button", {
    name: `${AGENT_NAME} agent profile`,
  });
  await expect(agentButton).toBeVisible({ timeout: 10_000 });
  await agentButton.click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible({
    timeout: 10_000,
  });
  await page.getByTestId("user-profile-edit-agent").click();
  await expect(page.getByTestId("edit-agent-dialog")).toBeVisible({
    timeout: 10_000,
  });
  // Wait for the runtime catalog to load and form to settle (buzz-agent auto-
  // expands Advanced, which makes the MCP servers editor immediately visible).
  await expect(page.getByTestId("mcp-servers-editor").first()).toBeVisible({
    timeout: 10_000,
  });
}

// ── round-trip tests ─────────────────────────────────────────────────────────

test("persona mcp_servers round-trip through create_persona + update_persona", async ({
  page,
}) => {
  await gotoApp(page);

  const fsServer = {
    name: "filesystem",
    command: "npx",
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
    env: [],
    enabled: true,
  };
  const fetchServer = {
    name: "fetch-mcp",
    command: "uvx",
    args: ["mcp-server-fetch"],
    env: [],
    enabled: true,
  };

  const created = await invokeTauri<{
    id: string;
    mcp_servers?: unknown[];
  }>(page, "create_persona", {
    input: {
      displayName: "MCP Persona",
      systemPrompt: "You use MCP.",
      mcpServers: [fsServer, fetchServer],
    },
  });

  expect(created.mcp_servers).toHaveLength(2);
  expect(created.mcp_servers?.[0]).toMatchObject({ name: "filesystem" });
  expect(created.mcp_servers?.[1]).toMatchObject({ name: "fetch-mcp" });

  // Update: drop one, change one, add one.
  const dbServer = {
    name: "sqlite-db",
    command: "uvx",
    args: ["mcp-server-sqlite", "--db", "/tmp/test.db"],
    env: [],
    enabled: true,
  };
  const updated = await invokeTauri<{ mcp_servers?: unknown[] }>(
    page,
    "update_persona",
    {
      input: {
        id: created.id,
        displayName: "MCP Persona",
        systemPrompt: "You use MCP.",
        mcpServers: [
          { ...fsServer, command: "bunx" }, // changed command
          dbServer, // added
          // fetch-mcp dropped
        ],
      },
    },
  );

  expect(updated.mcp_servers).toHaveLength(2);
  expect(updated.mcp_servers?.[0]).toMatchObject({
    name: "filesystem",
    command: "bunx",
  });
  expect(updated.mcp_servers?.[1]).toMatchObject({ name: "sqlite-db" });
});

test("update_persona preserves mcp_servers when caller omits the field", async ({
  page,
}) => {
  await gotoApp(page);

  const created = await invokeTauri<{
    id: string;
    mcp_servers?: unknown[];
  }>(page, "create_persona", {
    input: {
      displayName: "MCP Keeper",
      systemPrompt: "You keep MCP.",
      mcpServers: [
        {
          name: "filesystem",
          command: "npx",
          args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
          env: [],
          enabled: true,
        },
      ],
    },
  });
  expect(created.mcp_servers).toHaveLength(1);

  // Update WITHOUT including mcpServers. Stored list must survive.
  const preserved = await invokeTauri<{ mcp_servers?: unknown[] }>(
    page,
    "update_persona",
    {
      input: {
        id: created.id,
        displayName: "MCP Keeper (renamed)",
        systemPrompt: "You keep MCP.",
        // mcpServers intentionally omitted
      },
    },
  );
  expect(preserved.mcp_servers).toHaveLength(1);

  // Explicit empty array still clears (intentional).
  const cleared = await invokeTauri<{ mcp_servers?: unknown[] }>(
    page,
    "update_persona",
    {
      input: {
        id: created.id,
        displayName: "MCP Keeper (renamed)",
        systemPrompt: "You keep MCP.",
        mcpServers: [],
      },
    },
  );
  expect(cleared.mcp_servers ?? []).toHaveLength(0);
});

test("agent mcp_servers round-trip through create_managed_agent + update_managed_agent", async ({
  page,
}) => {
  await gotoApp(page);

  const created = await invokeTauri<{
    agent: { pubkey: string; mcp_servers?: unknown[] };
  }>(page, "create_managed_agent", {
    input: {
      name: "mcp-e2e-agent",
      backend: { type: "local" },
      mcpServers: [
        {
          name: "filesystem",
          command: "npx",
          args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
          env: [],
          enabled: true,
        },
      ],
    },
  });
  expect(created.agent.mcp_servers).toHaveLength(1);
  expect(created.agent.mcp_servers?.[0]).toMatchObject({
    name: "filesystem",
    command: "npx",
  });

  // Update mcpServers — replaces entire list.
  const updated = await invokeTauri<{
    agent: { mcp_servers?: unknown[] };
  }>(page, "update_managed_agent", {
    input: {
      pubkey: created.agent.pubkey,
      mcpServers: [
        {
          name: "fetch-mcp",
          command: "uvx",
          args: ["mcp-server-fetch"],
          env: [],
          enabled: true,
        },
      ],
    },
  });
  expect(updated.agent.mcp_servers).toHaveLength(1);
  expect(updated.agent.mcp_servers?.[0]).toMatchObject({
    name: "fetch-mcp",
    command: "uvx",
  });

  // Omitting mcpServers on a subsequent update preserves the stored list.
  const preserved = await invokeTauri<{
    agent: { mcp_servers?: unknown[] };
  }>(page, "update_managed_agent", {
    input: {
      pubkey: created.agent.pubkey,
      name: "mcp-e2e-agent-renamed",
      // mcpServers intentionally omitted
    },
  });
  expect(preserved.agent.mcp_servers).toHaveLength(1);
  expect(preserved.agent.mcp_servers?.[0]).toMatchObject({ name: "fetch-mcp" });
});

// ── UI tests ─────────────────────────────────────────────────────────────────

test("mcp servers editor renders in PersonaDialog new-persona form", async ({
  page,
}) => {
  await gotoApp(page);

  // Open the Agents view, click New > New agent.
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();

  const dialog = page.getByRole("dialog");

  // The default runtime is buzz-agent — Advanced auto-expands so the MCP
  // servers editor is immediately visible without a click.
  await expect(dialog.getByTestId("mcp-servers-editor")).toBeVisible({
    timeout: 10_000,
  });
  // Initially empty — no rows.
  await expect(dialog.getByTestId("mcp-servers-row")).toHaveCount(0);

  // Add a row and fill the name + command fields.
  await dialog.getByTestId("mcp-servers-add").click();
  await expect(dialog.getByTestId("mcp-servers-row")).toHaveCount(1);

  await dialog.getByTestId("mcp-servers-name").fill("filesystem");
  await dialog.getByTestId("mcp-servers-command").fill("npx");

  // Add a second row to verify multi-row rendering.
  await dialog.getByTestId("mcp-servers-add").click();
  await dialog.getByTestId("mcp-servers-name").last().fill("fetch-mcp");
  await dialog.getByTestId("mcp-servers-command").last().fill("uvx");

  await waitForAnimations(page);
  await dialog.screenshot({
    path: "test-results/mcp-servers-persona-dialog.png",
  });

  // Remove the first row — verify per-row removal works.
  await dialog.getByTestId("mcp-servers-remove").first().click();
  await expect(dialog.getByTestId("mcp-servers-row")).toHaveCount(1);
});

test("mcp servers editor renders in global agent config card", async ({
  page,
}) => {
  await installMockBridge(page, {
    globalAgentConfig: {
      env_vars: {},
      mcp_servers: [
        {
          name: "filesystem",
          command: "npx",
          args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
          env: [],
          enabled: true,
        },
      ],
      provider: null,
      model: null,
    },
  });

  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await expect(page.getByTestId("settings-global-agent-config")).toBeVisible({
    timeout: 10_000,
  });

  const card = page.getByTestId("settings-global-agent-config");

  // The McpServersEditor is inside the global config card.
  await expect(card.getByTestId("mcp-servers-editor")).toBeVisible({
    timeout: 5_000,
  });

  // The existing server from the mock shows as a pre-populated row.
  await expect(card.getByTestId("mcp-servers-row")).toHaveCount(1);
  await expect(card.getByTestId("mcp-servers-name")).toHaveValue("filesystem");

  await card.scrollIntoViewIfNeeded();
  await waitForAnimations(page);
  await card.screenshot({
    path: "test-results/mcp-servers-global-config-card.png",
  });
});

test("mcp servers editor renders in agent-instance edit dialog", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: AGENT_NAME,
        status: "stopped",
        channelNames: ["agents"],
        agentCommand: "buzz-agent",
        mcpServers: [
          {
            name: "filesystem",
            command: "npx",
            args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
            env: [],
            enabled: true,
          },
        ],
      },
    ],
  });

  await openEditDialog(page);

  const dialog = page.getByRole("dialog");

  // buzz-agent auto-expands Advanced — the MCP servers editor is already visible.
  const editor = dialog.getByTestId("mcp-servers-editor");
  await expect(editor).toBeVisible();

  // The agent's pre-existing MCP server renders as an editable row.
  await expect(editor.getByTestId("mcp-servers-row")).toHaveCount(1);
  await expect(editor.getByTestId("mcp-servers-name")).toHaveValue(
    "filesystem",
  );

  // Add a second row.
  await editor.getByTestId("mcp-servers-add").click();
  await editor.getByTestId("mcp-servers-name").last().fill("fetch-mcp");
  await editor.getByTestId("mcp-servers-command").last().fill("uvx");

  await waitForAnimations(page);
  await dialog.screenshot({
    path: "test-results/mcp-servers-edit-agent-dialog.png",
  });
});
