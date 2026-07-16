import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
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

async function openModelCombobox(
  page: import("@playwright/test").Page,
  model: import("@playwright/test").Locator,
) {
  // PersonaModelCombobox renders a role="combobox" trigger + a Radix Popover
  // with a search <input> and plain <button> options — not a role="menu".
  await model.click();
  const searchInput = page.getByPlaceholder("Search models…");
  await expect(searchInput).toBeVisible({ timeout: 5_000 });
  // Return the popover content container so callers can scope option clicks.
  return page.locator("[data-radix-popper-content-wrapper]").last();
}

async function selectDropdownOption(
  page: import("@playwright/test").Page,
  trigger: import("@playwright/test").Locator,
  optionName: string,
) {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    await expect(trigger).toBeVisible({ timeout: 2_000 });
    await trigger.press("Enter", { timeout: 2_000 });
    const menu = page
      .getByRole("menu")
      .filter({
        has: page.getByRole("menuitemradio", {
          exact: true,
          name: optionName,
        }),
      })
      .last();

    try {
      await expect(menu).toBeVisible({ timeout: 1_500 });
      await menu
        .getByRole("menuitemradio", { exact: true, name: optionName })
        .click({ force: true, timeout: 1_500 });
      return;
    } catch (error) {
      if (attempt === 1) throw error;
      await page.keyboard.press("Escape", { timeout: 1_000 });
      await waitForAnimations(page);
    }
  }
}

test("persona env_vars round-trip through create_persona + update_persona", async ({
  page,
}) => {
  await gotoApp(page);

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

  // Open the Agents view, click New > New agent to open the persona dialog.
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();

  // Scope all env-vars queries to the dialog: GlobalAgentConfigSettingsCard
  // also renders an EnvVarsEditor in the background settings pane (introduced
  // by this branch), so page-wide testid queries would match both.
  const dialog = page.getByRole("dialog");

  await dialog.getByRole("button", { name: "Advanced", exact: true }).click();
  await expect(dialog.getByTestId("env-vars-editor")).toBeVisible();
  // Initially empty (no rows — buzz-agent with no provider has no required keys).
  await expect(dialog.getByTestId("env-vars-key")).toHaveCount(0);

  // Add a row.
  await dialog.getByTestId("env-vars-add").click();
  await expect(dialog.getByTestId("env-vars-key")).toHaveCount(1);

  // Fill it in with realistic-looking keys/values to cover masked secrets and row controls.
  const keys = dialog.getByTestId("env-vars-key");
  const values = dialog.getByTestId("env-vars-value");
  await keys.nth(0).fill("ANTHROPIC_API_KEY");
  await values.nth(0).fill("sk-ant-abc");
  await dialog.getByTestId("env-vars-add").click();
  await keys.nth(1).fill("GOOSE_PROVIDER");
  await values.nth(1).fill("anthropic");
  await dialog.getByTestId("env-vars-add").click();
  await keys.nth(2).fill("OPENAI_BASE_URL");
  await values.nth(2).fill("https://api.openai.com/v1");

  // Capture a screenshot of the dialog with three env vars filled. Helps
  // reviewers see the UI at a glance.
  await waitForAnimations(page);
  await dialog.screenshot({ path: "test-results/persona-env-dialog.png" });

  // Remove the first row to verify per-row removal still works.
  await dialog.getByTestId("env-vars-remove").first().click();
  await expect(keys).toHaveCount(2);
});

test("persona model options follow the selected LLM provider", async ({
  page,
}) => {
  await gotoApp(page);

  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();

  const provider = page.locator("#persona-runtime");
  await page.getByRole("tab", { name: "Customize for this agent" }).click();
  const llmProvider = page.locator("#persona-llm-provider");
  const model = page.locator("#persona-model");
  await expect(provider).toContainText("Buzz Agent (default)");
  await expect(llmProvider).toBeVisible();
  await expect(model).toBeVisible();
  // Custom mode requires a model selection until a provider is chosen.
  await expect(model).toContainText("Choose a model");

  await selectDropdownOption(page, llmProvider, "OpenAI");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByLabel("OpenAI API Key")).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Advanced", exact: true }),
  ).toHaveAttribute("aria-expanded", "false");
  await expect(model).toBeVisible();
  // OpenAI requires an explicit model, so "Default model" is filtered out.
  // The combobox offers only "Custom model..." — verify it is present and selectable.
  const openAiModelPopover = await openModelCombobox(page, model);
  await openAiModelPopover
    .getByRole("button", { name: "Custom model...", exact: true })
    .click();

  await selectDropdownOption(page, llmProvider, "Anthropic");
  await expect(dialog.getByLabel("Anthropic API Key")).toBeVisible();
  await expect(dialog.getByLabel("OpenAI API Key")).not.toBeVisible();
  await expect(model).toBeVisible();

  // Switch back to inherited defaults — per-agent provider, credential, and
  // model controls disappear together.
  await page.getByRole("tab", { name: "Use AI defaults" }).click();
  await expect(llmProvider).not.toBeVisible();
  await expect(dialog.getByLabel("Anthropic API Key")).not.toBeVisible();
  await expect(model).not.toBeVisible();
});
