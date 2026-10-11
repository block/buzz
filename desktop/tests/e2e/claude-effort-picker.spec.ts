/**
 * Thinking-effort picker in the real Edit and Create dialogs for Claude Code.
 *
 * Levels never come from the stored session, which does not record the model
 * it ran; they come from Claude model data for the model that will run
 * (explicit, then global, then the adapter's reported default). A level the
 * saved model does not offer is never submitted, and only the agent's own
 * saved level shows as stored.
 */
import { expect, test, type Locator, type Page } from "@playwright/test";

import {
  installMockBridge,
  type MockBridgeOptions,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const AGENT_NAME = "Effort Agent";

const RUNTIMES = [
  ["claude", "Claude Code"],
  ["codex", "Codex"],
].map(([id, label]) => ({
  id,
  label,
  avatar_url: "",
  availability: "available",
  command: id,
  binary_path: `/mock/${id}`,
  default_args: [],
  mcp_command: null,
  install_hint: null,
  install_instructions_url: null,
  can_auto_install: false,
  requires_external_cli: true,
  underlying_cli_path: `/mock/${id}`,
  node_required: false,
  auth_status: { status: "authenticated" },
  source: "builtin",
}));

// The Claude adapter's catalog as reported by claude-agent-acp 0.36.1.
const CLAUDE_MODELS = {
  supportsSwitching: true,
  agentDefaultModel: "opus[1m]",
  models: [
    { id: "opus[1m]", name: "Opus" },
    { id: "claude-sonnet-5", name: "Sonnet" },
    { id: "haiku", name: "Haiku" },
  ],
};

/**
 * `origin` is the tier the reported effort resolved from; only `buzzExplicit`
 * is the agent's own saved level.
 */
function surface(
  effort?: { configId: string; options: string[] },
  storedEffort?: string,
  origin = "buzzExplicit",
) {
  return {
    runtimeId: "claude",
    runtimeLabel: "Claude Code",
    isPreSpawn: !effort,
    normalized: {
      model: null,
      provider: null,
      mode: null,
      thinkingEffort: storedEffort
        ? {
            value: storedEffort,
            origin,
            writeVia: {
              type: "respawnWithEnvVar",
              envKey: "BUZZ_ACP_EFFORT_LEVEL",
            },
            overriddenValue: null,
            overriddenOrigin: null,
            isRequired: false,
          }
        : null,
      maxOutputTokens: null,
      contextLimit: null,
      systemPrompt: null,
    },
    advanced: [],
    extensions: [],
    sources: {
      acpNative: "available",
      acpConfigOptions: "available",
      envVars: "notApplicable",
      configFile: "available",
      configFilePath: "~/.claude/settings.json",
      mcpConfigFilePath: "~/.claude.json",
    },
    ...(effort && {
      effortConfigId: effort.configId,
      effortOptions: effort.options.map((value) => ({ value })),
    }),
  };
}

const AGENT = {
  pubkey: TEST_IDENTITIES.tyler.pubkey,
  name: AGENT_NAME,
  runtime: "claude",
  status: "stopped" as const,
  channelNames: ["agents"],
};

async function install(page: Page, mock: MockBridgeOptions = {}) {
  await installMockBridge(page, {
    acpRuntimesCatalog: RUNTIMES,
    globalAgentConfig: {
      env_vars: {},
      provider: null,
      model: null,
      preferred_runtime: "claude",
    },
    discoverAgentModels: CLAUDE_MODELS,
    managedAgents: [AGENT],
    agentConfigSurface: surface(),
    ...mock,
  });
}

async function payloadsFor(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])
        .filter((entry) => entry.command === name)
        .map((entry) => entry.payload as { input: Record<string, unknown> }),
    command,
  );
}

async function menuValues(page: Page, trigger: Locator) {
  await trigger.click();
  const rows = await page.getByRole("menuitemradio").allTextContents();
  await page.keyboard.press("Escape");
  return rows;
}

async function pick(page: Page, trigger: Locator, name: string) {
  await trigger.press("Enter");
  const option = page.getByRole("menuitemradio", { name, exact: true });
  await expect(option).toBeVisible();
  // Keyboard selection avoids racing the menu's open animation.
  await option.press("Enter");
  await expect(trigger).toHaveAttribute("aria-expanded", "false");
}

const DISCOVERY_FAILED = "Could not load live models for this provider.";
const ADAPTER_LEVELS = ["Adapter default", "low", "medium", "high"];

/** A build-baked model, which never launches with Claude. */
function bakedModel(value: string) {
  return { bakedBuildEnv: [{ key: "BUZZ_AGENT_MODEL", value, masked: false }] };
}

/** Make the next discovery hang (`pending`) or reject (`failed`). */
async function stallDiscovery(page: Page, mode: "pending" | "failed") {
  await page.evaluate((nextMode) => {
    const mock = window.__BUZZ_E2E__?.mock;
    if (!mock) throw new Error("mock bridge missing");
    if (nextMode === "pending") mock.discoverAgentModelsDelayMs = 60_000;
    else mock.discoverAgentModelsError = "discovery failed";
  }, mode);
}

async function openEdit(page: Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page
    .getByRole("button", { name: `${AGENT_NAME} agent profile`, exact: true })
    .click();
  await page.getByTestId("user-profile-edit-agent").click();
  const dialog = page.getByTestId("edit-agent-dialog");
  await expect(dialog).toBeVisible();
  return dialog;
}

async function openCreate(page: Page) {
  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible();
  await dialog.getByLabel("Agent name").fill("Effort Create");
  await dialog.getByLabel("Agent instruction").fill("Review code.");
  await dialog.getByRole("button", { name: "Advanced", exact: true }).click();
  return dialog;
}

async function pickCreateModel(page: Page, dialog: Locator, name: string) {
  const customize = dialog.getByRole("tab", {
    name: "Customize for this agent",
  });
  if ((await customize.getAttribute("aria-selected")) !== "true") {
    await customize.click();
  }
  await dialog.locator("#persona-model").click();
  await page.getByRole("button", { name, exact: true }).click();
}

test.describe("edit dialog", () => {
  test("an untouched form ignores stored session levels from another model", async ({
    page,
  }) => {
    // The stored surface does not record which model the session ran; Opus
    // levels come from the model data, not the session's low/max.
    await install(page, {
      managedAgents: [{ ...AGENT, model: "opus[1m]" }],
      agentConfigSurface: surface({
        configId: "thought_level",
        options: ["low", "max"],
      }),
    });
    const dialog = await openEdit(page);
    const effort = dialog.locator("#edit-agent-effort");
    expect(await menuValues(page, effort)).toEqual([
      "Adapter default",
      "low",
      "medium",
      "high",
    ]);
    await pick(page, effort, "high");

    await pick(page, dialog.locator("#edit-agent-model"), "Haiku");
    await expect(effort).toHaveCount(0);

    await dialog.getByRole("button", { name: "Save changes" }).click();
    await expect(dialog).toHaveCount(0);
    const updates = await payloadsFor(page, "update_managed_agent");
    expect(updates).toHaveLength(1);
    expect(updates[0].input).not.toHaveProperty("effortLevel");
  });

  test("blank model offers the adapter default opus[1m] levels", async ({
    page,
  }) => {
    await install(page);
    const dialog = await openEdit(page);
    expect(
      await menuValues(page, dialog.locator("#edit-agent-effort")),
    ).toEqual(["Adapter default", "low", "medium", "high"]);
  });

  for (const baked of ["gpt-5.5", "claude-haiku-4-5"]) {
    test(`a baked ${baked} build model does not decide Claude levels`, async ({
      page,
    }) => {
      await install(page, bakedModel(baked));
      const dialog = await openEdit(page);
      expect(
        await menuValues(page, dialog.locator("#edit-agent-effort")),
      ).toEqual(ADAPTER_LEVELS);
    });
  }

  for (const mode of ["pending", "failed"] as const) {
    test(`a pick survives Save while rediscovery is ${mode}`, async ({
      page,
    }) => {
      await install(page, {
        managedAgents: [{ ...AGENT, envVars: { EFFORT_TEST: "one" } }],
      });
      const dialog = await openEdit(page);
      const effort = dialog.locator("#edit-agent-effort");
      await pick(page, effort, "high");

      await stallDiscovery(page, mode);
      await dialog
        .getByRole("button", { name: "Advanced", exact: true })
        .click();
      await dialog.getByTestId("env-vars-value").first().fill("two");
      await expect(effort).toHaveCount(0);
      if (mode === "failed")
        await expect(dialog).toContainText(DISCOVERY_FAILED);

      await dialog.getByRole("button", { name: "Save changes" }).click();
      await expect(dialog).toHaveCount(0);
      const updates = await payloadsFor(page, "update_managed_agent");
      expect(updates).toHaveLength(1);
      expect(updates[0].input.effortLevel).toBe("high");
    });
  }

  test("a stored level the new model doesn't list stays visible and clearable", async ({
    page,
  }) => {
    await install(page, {
      managedAgents: [{ ...AGENT, model: "claude-opus-4-8" }],
      agentConfigSurface: surface(undefined, "max"),
      discoverAgentModels: {
        ...CLAUDE_MODELS,
        models: [
          ...CLAUDE_MODELS.models,
          { id: "claude-opus-4-8", name: "Opus 4.8" },
        ],
      },
    });
    const dialog = await openEdit(page);
    const effort = dialog.locator("#edit-agent-effort");
    const model = dialog.locator("#edit-agent-model");
    await expect(effort).toHaveText("max");

    await pick(page, model, "Opus");
    await expect(effort).toHaveText("max");
    await expect(dialog).toContainText("This model may not support max.");

    await pick(page, model, "Haiku");
    await expect(effort).toHaveText("max");
    expect(await menuValues(page, effort)).toEqual(["Adapter default", "max"]);

    await dialog.getByRole("button", { name: "Save changes" }).click();
    await expect(dialog).toHaveCount(0);
    const updates = await payloadsFor(page, "update_managed_agent");
    expect(updates).toHaveLength(1);
    expect(updates[0].input).not.toHaveProperty("effortLevel");
  });

  test("clearing an unlisted stored level saves the adapter default", async ({
    page,
  }) => {
    await install(page, {
      managedAgents: [{ ...AGENT, model: "haiku" }],
      agentConfigSurface: surface(undefined, "max"),
    });
    const dialog = await openEdit(page);
    const effort = dialog.locator("#edit-agent-effort");
    await expect(effort).toHaveText("max");
    await pick(page, effort, "Adapter default");
    await expect(effort).toHaveText("Adapter default");
    await expect(dialog).not.toContainText("This model may not support max.");

    await dialog.getByRole("button", { name: "Save changes" }).click();
    await expect(dialog).toHaveCount(0);
    const updates = await payloadsFor(page, "update_managed_agent");
    expect(updates).toHaveLength(1);
    expect(updates[0].input).toHaveProperty("effortLevel", null);
  });

  for (const clear of [false, true]) {
    test(`failed discovery keeps a stored level visible (${clear ? "cleared" : "untouched"})`, async ({
      page,
    }) => {
      await install(page, {
        agentConfigSurface: surface(undefined, "max"),
        discoverAgentModelsError: "discovery failed",
      });
      const dialog = await openEdit(page);
      const effort = dialog.locator("#edit-agent-effort");
      await expect(dialog).toContainText(DISCOVERY_FAILED);
      await expect(effort).toHaveText("max");
      await expect(dialog).toContainText("Support for max isn't known yet.");
      expect(await menuValues(page, effort)).toEqual([
        "Adapter default",
        "max",
      ]);
      if (clear) {
        await pick(page, effort, "Adapter default");
        await expect(effort).toHaveText("Adapter default");
      }

      await dialog.getByRole("button", { name: "Save changes" }).click();
      await expect(dialog).toHaveCount(0);
      const updates = await payloadsFor(page, "update_managed_agent");
      expect(updates).toHaveLength(1);
      if (clear) expect(updates[0].input).toHaveProperty("effortLevel", null);
      else expect(updates[0].input).not.toHaveProperty("effortLevel");
    });

    test(`a runtime round trip keeps a stored level visible (${clear ? "cleared" : "untouched"})`, async ({
      page,
    }) => {
      await install(page, {
        managedAgents: [{ ...AGENT, model: "claude-opus-4-8" }],
        agentConfigSurface: surface(undefined, "max"),
        discoverAgentModels: {
          ...CLAUDE_MODELS,
          models: [
            ...CLAUDE_MODELS.models,
            { id: "claude-opus-4-8", name: "Opus 4.8" },
          ],
        },
      });
      const dialog = await openEdit(page);
      const effort = dialog.locator("#edit-agent-effort");
      const runtime = dialog.locator("#edit-agent-runtime");
      const model = dialog.locator("#edit-agent-model");
      await expect(effort).toHaveText("max");
      await pick(page, runtime, "Codex");
      await pick(page, runtime, "Claude Code");
      await pick(page, model, "Opus");
      await expect(effort).toHaveText("max");
      await pick(page, model, "Haiku");
      await expect(effort).toHaveText("max");
      await expect(dialog).toContainText("This model may not support max.");
      if (clear) {
        await pick(page, effort, "Adapter default");
        await expect(effort).toHaveText("Adapter default");
      }

      await dialog.getByRole("button", { name: "Save changes" }).click();
      await expect(dialog).toHaveCount(0);
      const updates = await payloadsFor(page, "update_managed_agent");
      expect(updates).toHaveLength(1);
      if (clear) expect(updates[0].input).toHaveProperty("effortLevel", null);
      else expect(updates[0].input).not.toHaveProperty("effortLevel");
    });
  }

  test("a config-file effort is not shown as the agent's stored level", async ({
    page,
  }) => {
    // ~/.claude/settings.json effortLevel reaches the surface as configFile.
    await install(page, {
      agentConfigSurface: surface(undefined, "max", "configFile"),
    });
    const dialog = await openEdit(page);
    const effort = dialog.locator("#edit-agent-effort");
    await expect(effort).toHaveText("Adapter default");
    expect(await menuValues(page, effort)).toEqual(ADAPTER_LEVELS);
  });

  test("a config-file effort shows no picker for Haiku", async ({ page }) => {
    await install(page, {
      managedAgents: [{ ...AGENT, model: "haiku" }],
      agentConfigSurface: surface(undefined, "max", "configFile"),
    });
    const dialog = await openEdit(page);
    await expect(dialog.locator("#edit-agent-model")).toHaveText("Haiku");
    await expect(dialog.locator("#edit-agent-effort")).toHaveCount(0);
  });

  test("a pick the new model drops shows the stored level Save keeps", async ({
    page,
  }) => {
    await install(page, {
      managedAgents: [{ ...AGENT, model: "claude-opus-4-8" }],
      agentConfigSurface: surface(undefined, "max"),
      discoverAgentModels: {
        ...CLAUDE_MODELS,
        models: [
          ...CLAUDE_MODELS.models,
          { id: "claude-opus-4-8", name: "Opus 4.8" },
        ],
      },
    });
    const dialog = await openEdit(page);
    const effort = dialog.locator("#edit-agent-effort");
    await pick(page, effort, "high");
    await pick(page, dialog.locator("#edit-agent-model"), "Haiku");
    await expect(effort).toHaveText("max");
    await expect(dialog).toContainText("This model may not support max.");

    await dialog.getByRole("button", { name: "Save changes" }).click();
    await expect(dialog).toHaveCount(0);
    const updates = await payloadsFor(page, "update_managed_agent");
    expect(updates).toHaveLength(1);
    expect(updates[0].input).not.toHaveProperty("effortLevel");
  });

  for (const clear of [false, true]) {
    test(`a stored level survives a switch to another runtime (${clear ? "cleared" : "untouched"})`, async ({
      page,
    }) => {
      // The backend keeps effort_level across a runtime switch.
      await install(page, {
        managedAgents: [{ ...AGENT, runtime: "codex" }],
        agentConfigSurface: surface(undefined, "xhigh"),
      });
      const dialog = await openEdit(page);
      const effort = dialog.locator("#edit-agent-effort");
      await expect(effort).toHaveText("xhigh");
      await pick(page, dialog.locator("#edit-agent-runtime"), "Claude Code");
      await expect(effort).toHaveText("xhigh");
      await expect(dialog).toContainText("This model may not support xhigh.");
      expect(await menuValues(page, effort)).toEqual([
        ...ADAPTER_LEVELS,
        "xhigh",
      ]);
      if (clear) {
        await pick(page, effort, "Adapter default");
        await expect(effort).toHaveText("Adapter default");
      }

      await dialog.getByRole("button", { name: "Save changes" }).click();
      await expect(dialog).toHaveCount(0);
      const updates = await payloadsFor(page, "update_managed_agent");
      expect(updates).toHaveLength(1);
      expect(updates[0].input.agentCommand).toBe("claude");
      if (clear) expect(updates[0].input).toHaveProperty("effortLevel", null);
      else expect(updates[0].input).not.toHaveProperty("effortLevel");
    });
  }

  test("a runtime switch while discovery is pending leaks no Claude levels", async ({
    page,
  }) => {
    await install(page, { discoverAgentModelsDelayMs: 1_500 });
    const dialog = await openEdit(page);
    const effort = dialog.locator("#edit-agent-effort");
    await expect(dialog.locator("#edit-agent-model")).toBeVisible();
    await expect(effort).toHaveCount(0);

    await pick(page, dialog.locator("#edit-agent-runtime"), "Codex");
    await page.waitForTimeout(2_000);
    await expect(effort).toHaveCount(0);
  });
});

test.describe("create dialog", () => {
  test("local create-and-start sends the picked effort", async ({ page }) => {
    await install(page);
    const dialog = await openCreate(page);
    await pickCreateModel(page, dialog, "Opus");
    await pick(page, dialog.locator("#edit-agent-effort"), "high");
    await dialog.getByRole("button", { name: "Add agent" }).click();
    await expect(dialog).toHaveCount(0);
    const creates = await payloadsFor(page, "create_managed_agent");
    expect(creates).toHaveLength(1);
    expect(creates[0].input.effortLevel).toBe("high");
  });

  for (const baked of ["gpt-5.5", "claude-haiku-4-5"]) {
    test(`a baked ${baked} build model does not decide Claude levels`, async ({
      page,
    }) => {
      await install(page, bakedModel(baked));
      const dialog = await openCreate(page);
      expect(
        await menuValues(page, dialog.locator("#edit-agent-effort")),
      ).toEqual(ADAPTER_LEVELS);
    });
  }

  for (const mode of ["pending", "failed"] as const) {
    test(`a pick survives create while rediscovery is ${mode}`, async ({
      page,
    }) => {
      await install(page);
      const dialog = await openCreate(page);
      const effort = dialog.locator("#edit-agent-effort");
      await pick(page, effort, "high");

      await stallDiscovery(page, mode);
      await dialog.getByTestId("env-vars-add").click();
      await dialog.getByTestId("env-vars-key").last().fill("EFFORT_TEST");
      await dialog.getByTestId("env-vars-value").last().fill("two");
      await expect(effort).toHaveCount(0);
      // Create shows no discovery status; wait for the failing call instead.
      await expect
        .poll(async () =>
          JSON.stringify(await payloadsFor(page, "discover_agent_models")),
        )
        .toContain("two");

      await dialog.getByRole("button", { name: "Add agent" }).click();
      await expect(dialog).toHaveCount(0);
      const creates = await payloadsFor(page, "create_managed_agent");
      expect(creates).toHaveLength(1);
      expect(creates[0].input.effortLevel).toBe("high");
    });
  }

  test("switching to Haiku hides the picker and sends no effort", async ({
    page,
  }) => {
    await install(page);
    const dialog = await openCreate(page);
    await pickCreateModel(page, dialog, "Opus");
    await pick(page, dialog.locator("#edit-agent-effort"), "high");
    await pickCreateModel(page, dialog, "Haiku");
    await expect(dialog.locator("#edit-agent-effort")).toHaveCount(0);
    await dialog.getByRole("button", { name: "Add agent" }).click();
    await expect(dialog).toHaveCount(0);
    const creates = await payloadsFor(page, "create_managed_agent");
    expect(creates).toHaveLength(1);
    expect(creates[0].input).not.toHaveProperty("effortLevel");
  });

  test("switching to Haiku with Advanced collapsed sends no effort", async ({
    page,
  }) => {
    await install(page);
    const dialog = await openCreate(page);
    await pickCreateModel(page, dialog, "Opus");
    await pick(page, dialog.locator("#edit-agent-effort"), "high");
    await dialog.getByRole("button", { name: "Advanced", exact: true }).click();
    await expect(dialog.locator("#edit-agent-effort")).toHaveCount(0);
    await pickCreateModel(page, dialog, "Haiku");
    await dialog.getByRole("button", { name: "Add agent" }).click();
    await expect(dialog).toHaveCount(0);
    const creates = await payloadsFor(page, "create_managed_agent");
    expect(creates).toHaveLength(1);
    expect(creates[0].input).not.toHaveProperty("effortLevel");
  });

  test("a remote create hides the picker and sends no effort", async ({
    page,
  }) => {
    await install(page, {
      backendProviders: [
        { id: "kubernetes", binaryPath: "/mock/buzz-backend-kubernetes" },
      ],
      backendProviderProbeResult: {
        ok: true,
        name: "kubernetes",
        version: "0.0.0-mock",
        config_schema: {
          type: "object",
          properties: { namespace: { type: "string", default: "agents" } },
          required: ["namespace"],
        },
      },
    });
    const dialog = await openCreate(page);
    await pickCreateModel(page, dialog, "Opus");
    await pick(page, dialog.locator("#edit-agent-effort"), "high");
    await pick(page, dialog.locator("#agent-run-on"), "kubernetes");
    await expect(dialog.locator("#provider-cfg-namespace")).toHaveValue(
      "agents",
    );
    await expect(dialog.locator("#edit-agent-effort")).toHaveCount(0);
    await dialog.getByRole("button", { name: "Add agent" }).click();
    await expect(dialog).toHaveCount(0);
    const creates = await payloadsFor(page, "create_managed_agent");
    expect(creates).toHaveLength(1);
    expect(creates[0].input).not.toHaveProperty("effortLevel");
  });

  test("editing a definition shows no picker and saves no effort", async ({
    page,
  }) => {
    await install(page, {
      personas: [
        {
          id: "persona-opus",
          displayName: "Opus Definition",
          systemPrompt: "Original.",
          runtime: "claude",
          model: "opus[1m]",
        },
      ],
      activePersonaIds: ["persona-opus"],
    });
    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    await page.getByLabel("Open actions for Opus Definition").click();
    await page.getByRole("menuitem", { name: "Edit" }).click();
    const dialog = page.getByTestId("persona-dialog");
    await dialog.getByRole("button", { name: "Advanced", exact: true }).click();
    await expect(dialog.getByTestId("agent-respond-to")).toBeVisible();
    await expect(dialog.locator("#edit-agent-effort")).toHaveCount(0);
    await dialog.getByLabel("Agent instruction").fill("Edited.");
    await dialog.getByRole("button", { name: "Save changes" }).click();
    await expect(dialog).toHaveCount(0);
    const saves = await page.evaluate(() =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter((entry) =>
        /persona|managed_agent/.test(entry.command),
      ),
    );
    expect(JSON.stringify(saves)).not.toContain("effortLevel");
  });
});
