import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

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

test("persona env_vars round-trip through create_persona + update_persona", async ({
  page,
}) => {
  await gotoApp(page);

  // Create a persona with two env vars.
  const created = await invokeTauri<{
    id: string;
    env_vars?: Record<string, string>;
  }>(page, "create_persona", {
    input: {
      displayName: "Coder",
      systemPrompt: "You are Coder.",
      envVars: {
        ANTHROPIC_API_KEY: "sk-ant-test-001",
        ANTHROPIC_MODEL: "claude-sonnet-4-5",
      },
    },
  });

  expect(created.env_vars).toEqual({
    ANTHROPIC_API_KEY: "sk-ant-test-001",
    ANTHROPIC_MODEL: "claude-sonnet-4-5",
  });

  // Update: drop one, add one, change one.
  const updated = await invokeTauri<{ env_vars?: Record<string, string> }>(
    page,
    "update_persona",
    {
      input: {
        id: created.id,
        displayName: "Coder",
        systemPrompt: "You are Coder.",
        envVars: {
          ANTHROPIC_API_KEY: "sk-ant-test-002", // changed
          OPENAI_COMPAT_API_KEY: "sk-new", // added
          // ANTHROPIC_MODEL dropped
        },
      },
    },
  );

  expect(updated.env_vars).toEqual({
    ANTHROPIC_API_KEY: "sk-ant-test-002",
    OPENAI_COMPAT_API_KEY: "sk-new",
  });

  // list_personas returns the updated map.
  const list = await invokeTauri<
    Array<{ id: string; env_vars?: Record<string, string> }>
  >(page, "list_personas");
  const reloaded = list.find((p) => p.id === created.id);
  expect(reloaded?.env_vars).toEqual({
    ANTHROPIC_API_KEY: "sk-ant-test-002",
    OPENAI_COMPAT_API_KEY: "sk-new",
  });
});

test("update_persona preserves env_vars when caller omits the field", async ({
  page,
}) => {
  // Regression: codex R2 P1. Editing display_name / system_prompt via a
  // helper that doesn't populate envVars must NOT wipe stored credentials.
  await gotoApp(page);

  const created = await invokeTauri<{
    id: string;
    env_vars?: Record<string, string>;
  }>(page, "create_persona", {
    input: {
      displayName: "WithSecrets",
      systemPrompt: "You have secrets.",
      envVars: {
        ANTHROPIC_API_KEY: "sk-keep-me",
      },
    },
  });
  expect(created.env_vars).toEqual({ ANTHROPIC_API_KEY: "sk-keep-me" });

  // Update WITHOUT including envVars at all. Stored value must survive.
  const updated = await invokeTauri<{ env_vars?: Record<string, string> }>(
    page,
    "update_persona",
    {
      input: {
        id: created.id,
        displayName: "WithSecrets (renamed)",
        systemPrompt: "You have secrets.",
        // envVars intentionally omitted
      },
    },
  );
  expect(updated.env_vars).toEqual({ ANTHROPIC_API_KEY: "sk-keep-me" });

  // Explicit empty map still clears (intentional).
  const cleared = await invokeTauri<{ env_vars?: Record<string, string> }>(
    page,
    "update_persona",
    {
      input: {
        id: created.id,
        displayName: "WithSecrets (renamed)",
        systemPrompt: "You have secrets.",
        envVars: {},
      },
    },
  );
  expect(cleared.env_vars ?? {}).toEqual({});
});

test("agent env_vars override persona env_vars on the agent record", async ({
  page,
}) => {
  await gotoApp(page);

  const persona = await invokeTauri<{ id: string }>(page, "create_persona", {
    input: {
      displayName: "Provider",
      systemPrompt: "You are Provider.",
      envVars: {
        ANTHROPIC_API_KEY: "persona-key",
        SHARED_VAR: "from-persona",
      },
    },
  });

  // Create an agent under that persona with an override.
  const created = await invokeTauri<{
    agent: { pubkey: string; env_vars?: Record<string, string> };
  }>(page, "create_managed_agent", {
    input: {
      name: "agent-with-overrides",
      personaId: persona.id,
      backend: { type: "local" },
      envVars: {
        ANTHROPIC_API_KEY: "agent-key", // overrides persona
        AGENT_ONLY: "1", // agent-only
      },
    },
  });

  expect(created.agent.env_vars).toEqual({
    ANTHROPIC_API_KEY: "agent-key",
    AGENT_ONLY: "1",
  });

  // Update to replace the map entirely.
  const updated = await invokeTauri<{
    agent: { env_vars?: Record<string, string> };
  }>(page, "update_managed_agent", {
    input: {
      pubkey: created.agent.pubkey,
      envVars: {
        AGENT_ONLY: "2",
      },
    },
  });

  expect(updated.agent.env_vars).toEqual({ AGENT_ONLY: "2" });
});

test("env vars editor renders in PersonaDialog new-persona form", async ({
  page,
}) => {
  await gotoApp(page);

  // Open the Agents view, click New > Persona to open the persona dialog.
  await page.getByTestId("open-agents-view").click();
  await page
    .getByTestId("agents-library-personas")
    .getByRole("button", { name: "New", exact: true })
    .click();
  await page.getByRole("menuitem", { name: /^Persona$/ }).click();

  // The env vars editor should be present.
  await expect(page.getByTestId("env-vars-editor")).toBeVisible();
  // Initially empty (no rows).
  await expect(page.getByTestId("env-vars-key")).toHaveCount(0);

  // Add a row.
  await page.getByTestId("env-vars-add").click();
  await expect(page.getByTestId("env-vars-key")).toHaveCount(1);

  // Fill it in. Use realistic-looking keys/values so the screenshot
  // captured below illustrates the feature for reviewers.
  const keys = page.getByTestId("env-vars-key");
  const values = page.getByTestId("env-vars-value");
  await keys.nth(0).fill("ANTHROPIC_API_KEY");
  await values.nth(0).fill("sk-ant-abc");
  await page.getByTestId("env-vars-add").click();
  await keys.nth(1).fill("GOOSE_PROVIDER");
  await values.nth(1).fill("anthropic");
  await page.getByTestId("env-vars-add").click();
  await keys.nth(2).fill("OPENAI_BASE_URL");
  await values.nth(2).fill("https://api.openai.com/v1");

  // Capture a screenshot of the dialog with three env vars filled. Helps
  // reviewers see the UI at a glance.
  await page
    .getByRole("dialog")
    .screenshot({ path: "test-results/persona-env-dialog.png" });

  // Remove the first row to verify per-row removal still works.
  await page.getByTestId("env-vars-remove").first().click();
  await expect(keys).toHaveCount(2);
});
