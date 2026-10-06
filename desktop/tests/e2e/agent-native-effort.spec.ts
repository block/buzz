import { expect, test, type Page } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// Synthetic ACP responses exercise the production form and persistence path.
// Real adapter acceptance is recorded separately; these fixtures make no
// assertion about the models available on any contributor's account.
const models = [
  ["codex", "gpt-6-astra"],
  ["codex", "gpt-6-sol"],
  ["codex", "gpt-6-luna"],
  ["claude", "claude-opus-5-5"],
  ["claude", "claude-sonnet-5-5"],
];
const catalog = ["codex", "claude"].map((id) => ({
  id,
  label: id === "codex" ? "Codex" : "Claude Code",
  availability: "available",
  command: `${id}-acp`,
  binary_path: `/test/${id}-acp`,
  default_args: [],
  avatar_url: "",
  mcp_command: null,
  model_env_var: null,
  provider_env_var: null,
  thinking_env_var: "BUZZ_ACP_EFFORT_LEVEL",
  required_env_vars: [],
  install_hint: "Test fixture",
  install_instructions_url: "https://github.com/block/buzz",
  can_auto_install: false,
  underlying_cli_path: null,
  auth_status: { status: "authenticated" },
}));

async function savedConfig(page: Page) {
  return page.evaluate(async () =>
    (
      window as typeof window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<unknown>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null),
  );
}

for (const [runtime, model] of models) {
  test(`${model}: change harness, save low/medium/high, and reopen`, async ({
    page,
  }) => {
    await installMockBridge(page, {
      globalAgentConfig: {
        preferred_runtime: runtime === "codex" ? "claude" : "codex",
        provider: null,
        model: null,
        env_vars: {},
      },
      acpRuntimesCatalog: catalog,
      discoverAgentModels: {
        models: models.map(([, id]) => ({ id, name: id })),
        supportsSwitching: true,
        effortOption: {
          id: "effort",
          category: "thought_level",
          options: ["default", "low", "medium", "high"].map((value) => ({
            value,
          })),
        },
      },
    });
    await page.goto("/");
    await page.getByTestId("open-settings").click();
    await page.getByTestId("profile-popover-settings").click();
    await page.getByTestId("settings-nav-agents").click();
    const card = page.getByTestId("settings-global-agent-config");
    await card.getByTestId("global-agent-default-harness").click();
    await page
      .getByTestId(`global-agent-default-harness-option-${runtime}`)
      .click();
    await waitForAnimations(page);
    const modelPicker = card
      .getByTestId("global-agent-model")
      .filter({ visible: true });
    await expect(modelPicker).toHaveCount(1);
    await modelPicker.click();
    await page.getByTestId(`global-agent-model-option-${model}`).click();
    const effort = card
      .getByTestId("global-agent-thinking-effort-select")
      .filter({ visible: true });
    for (const level of ["low", "medium", "high"]) {
      await expect(effort).toBeEnabled();
      await effort.click();
      await page
        .getByTestId(`global-agent-thinking-effort-select-option-${level}`)
        .click();
      await card.getByRole("button", { name: "Save defaults" }).click();
      await expect
        .poll(() => savedConfig(page))
        .toMatchObject({
          preferred_runtime: runtime,
          model,
          env_vars: { BUZZ_ACP_EFFORT_LEVEL: level },
        });
    }
    await page.getByTestId("settings-nav-appearance").click();
    await page.getByTestId("settings-nav-agents").click();
    await expect(
      card.getByTestId("global-agent-model").filter({ visible: true }),
    ).toHaveAttribute("data-value", model);
    await expect(effort).toContainText(/high/i);
    await waitForAnimations(page);
    await card.screenshot({ path: `test-results/native-effort/${model}.png` });
  });
}
