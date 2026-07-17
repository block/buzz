import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";
import { passThroughBackupStep } from "../helpers/onboarding";

const SHOTS = "test-results/screenshots-onboarding";

function availableRuntime(
  id: "buzz-agent" | "claude" | "codex" | "goose",
  authStatus:
    | { status: "config_invalid"; diagnostic: string }
    | { status: "logged_in" | "logged_out" | "not_applicable" | "unknown" },
  overrides: Record<string, unknown> = {},
) {
  return {
    id,
    label:
      id === "buzz-agent"
        ? "Buzz Agent"
        : id === "claude"
          ? "Claude"
          : id === "codex"
            ? "Codex"
            : "Goose",
    avatar_url: "",
    availability: "available",
    command: id,
    binary_path: `/usr/local/bin/${id}`,
    default_args: [],
    mcp_command: null,
    install_hint: "",
    install_instructions_url: "https://example.com",
    can_auto_install: false,
    underlying_cli_path: null,
    node_required: false,
    auth_status: authStatus,
    login_hint: `Sign in to ${id}`,
    ...overrides,
  };
}

/** Drive to the harness setup page (page 3) via the full onboarding flow. */
async function navigateToSetupPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await page.getByRole("button", { name: "Get started" }).click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
}

/** Drive to the default config page (page 4), past the harness page. */
async function navigateToConfigPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await navigateToSetupPage(page);
  await page.getByTestId("onboarding-runtime-buzz-agent").click();
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
}

async function chooseConfigDropdownOption(
  page: Parameters<typeof installMockBridge>[0],
  triggerTestId: string,
  value: string,
) {
  await page.getByTestId(triggerTestId).click();
  await page.getByTestId(`${triggerTestId}-option-${value || "empty"}`).click();
}

async function readSavedRuntime(page: Parameters<typeof installMockBridge>[0]) {
  const savedConfig = await page.evaluate(() =>
    (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<{ preferred_runtime?: string | null }>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null),
  );
  return savedConfig?.preferred_runtime ?? null;
}

test("requires a runtime selection and routes Buzz Agent to config", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("buzz-agent", { status: "not_applicable" }),
      ],
      setGlobalAgentConfigDelayMs: 200,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const next = page.getByTestId("onboarding-setup-next");
  const card = page.getByTestId("onboarding-runtime-buzz-agent");
  await expect(next).toBeDisabled();
  await expect(card.getByText("Preferred")).toHaveCount(0);

  await card.click();
  await expect(next).toHaveText("Saving…");
  await expect(next).toBeDisabled();
  await expect(card).toHaveAttribute("aria-checked", "true");
  await expect(next).toBeEnabled();
  await next.click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("buzz-agent");
});

test("authenticated Claude saves the selected runtime and skips detailed config", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [availableRuntime("claude", { status: "logged_in" })],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await page.getByTestId("onboarding-runtime-claude").click();
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("machine-onboarding-gate")).toHaveCount(0);
  expect(await readSavedRuntime(page)).toBe("claude");
});

test("successful Claude sign-in selects the card without extra status copy", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_out" }),
      ],
      acpAuthMethods: {
        claude: {
          methods: [
            {
              id: "claude-login",
              name: "Sign in",
              description: null,
              type: "terminal",
            },
          ],
        },
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await waitForAnimations(page);

  const card = page.getByTestId("onboarding-runtime-claude");
  await card.getByRole("button", { name: "Sign in" }).click({ force: true });

  await expect(card).toHaveAttribute("aria-checked", "true");
  await expect(card.getByText("Preferred")).toHaveCount(0);
  await expect(
    page.getByTestId("onboarding-runtime-checkmark-claude"),
  ).toHaveCSS("opacity", "1");
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
  expect(await readSavedRuntime(page)).toBe("claude");
});

test("successful Codex sign-in hides API key auth and selects the card", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [availableRuntime("codex", { status: "logged_out" })],
      acpAuthMethods: {
        codex: {
          methods: [
            {
              id: "api-key",
              name: "Use API key",
              description: null,
              type: "input",
            },
            {
              id: "chat-gpt",
              name: "Sign in with ChatGPT",
              description: null,
              type: "browser",
            },
          ],
        },
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await waitForAnimations(page);

  const card = page.getByTestId("onboarding-runtime-codex");
  await expect(card.getByRole("button", { name: "Use API key" })).toHaveCount(
    0,
  );
  await expect(
    card.getByRole("button", { name: "Sign in with ChatGPT" }),
  ).toHaveCount(0);

  await card.getByRole("button", { name: "Log in" }).click({ force: true });
  await expect(card).toHaveAttribute("aria-checked", "true");
  await expect(card.getByText("Preferred")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
  expect(await readSavedRuntime(page)).toBe("codex");
});

test("runtime selection is gated by availability and auth state", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_out" }),
        availableRuntime("codex", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await expect(page.getByTestId("onboarding-runtime-claude")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await expect(page.getByTestId("onboarding-runtime-codex")).toHaveAttribute(
    "aria-disabled",
    "false",
  );
});

test("logged-out available runtimes show setup instead of installed", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_out" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const card = page.getByTestId("onboarding-runtime-claude");
  await expect(card.getByText("SET UP")).toBeVisible();
  await expect(
    page.getByTestId("onboarding-runtime-installed-claude"),
  ).toHaveCount(0);
});

test("runtime cards use the selected onboarding order", async ({ page }) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("buzz-agent", { status: "not_applicable" }),
        availableRuntime("goose", { status: "not_applicable" }),
        availableRuntime("codex", { status: "logged_in" }),
        availableRuntime("claude", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const runtimeOrder = await page
    .getByRole("checkbox")
    .evaluateAll((cards) =>
      cards.map((card) => card.getAttribute("data-testid")),
    );
  expect(runtimeOrder).toEqual([
    "onboarding-runtime-claude",
    "onboarding-runtime-codex",
    "onboarding-runtime-goose",
    "onboarding-runtime-buzz-agent",
  ]);
});

test("runtime cards allow multiple harness selections", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");
  await navigateToSetupPage(page);

  const next = page.getByTestId("onboarding-setup-next");
  const gooseCard = page.getByTestId("onboarding-runtime-goose");
  const buzzCard = page.getByTestId("onboarding-runtime-buzz-agent");

  await expect(next).toBeDisabled();

  await gooseCard.click();
  await expect(gooseCard).toHaveAttribute("aria-checked", "true");
  await expect(next).toBeEnabled();

  await buzzCard.click();
  await expect(gooseCard).toHaveAttribute("aria-checked", "true");
  await expect(buzzCard).toHaveAttribute("aria-checked", "true");
  await expect(next).toBeEnabled();

  await gooseCard.click();
  await expect(gooseCard).toHaveAttribute("aria-checked", "false");
  await expect(buzzCard).toHaveAttribute("aria-checked", "true");
  await expect(next).toBeEnabled();
  expect(await readSavedRuntime(page)).toBe("buzz-agent");
});

for (const authStatus of [
  { status: "logged_out" as const },
  { status: "unknown" as const },
  { status: "config_invalid" as const, diagnostic: "Fix Claude config" },
]) {
  test(`Claude ${authStatus.status} state cannot be selected directly`, async ({
    page,
  }) => {
    await installMockBridge(
      page,
      {
        acpRuntimesCatalog: [availableRuntime("claude", authStatus)],
        acpAuthMethods: {
          claude: {
            methods: [
              {
                id: "login",
                name: "Sign in",
                description: null,
                type: "terminal",
              },
            ],
          },
        },
      },
      { skipCommunitySeed: true, skipOnboardingSeed: true },
    );
    await page.goto("/");
    await navigateToSetupPage(page);

    const card = page.getByTestId("onboarding-runtime-claude");
    await expect(card).toHaveAttribute("aria-disabled", "true");
    if (authStatus.status === "logged_out") {
      await expect(card.getByRole("button", { name: "Sign in" })).toBeVisible();
    } else if (authStatus.status === "unknown") {
      await expect(
        card.getByText("Couldn’t verify authentication"),
      ).toBeVisible();
    } else {
      await expect(card.getByText("Fix Claude config")).toBeVisible();
    }
  });
}

test("setup cards only show checks after user selection", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToSetupPage(page);

  const gooseCard = page.getByTestId("onboarding-runtime-goose");
  const gooseCheck = page.getByTestId("onboarding-runtime-check-goose");
  const gooseCheckmark = page.getByTestId("onboarding-runtime-checkmark-goose");

  await expect(gooseCard).toHaveAttribute("aria-checked", "false");
  await expect(gooseCheckmark).toHaveCSS("opacity", "0");
  await expect(gooseCheck).toHaveCSS("opacity", "0");

  await gooseCard.hover();
  await expect(gooseCheck).toHaveCSS("opacity", "1");
  await expect(gooseCheckmark).toHaveCSS("opacity", "0");

  await gooseCard.click();
  await expect(gooseCard).toHaveAttribute("aria-checked", "true");
  await expect(gooseCheckmark).toHaveCSS("opacity", "1");
  await expect(
    page.getByTestId("onboarding-runtime-installed-goose"),
  ).toHaveText("INSTALLED");
});

test("unready setup pill runs setup from keyboard", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToSetupPage(page);

  const claudeCard = page.getByTestId("onboarding-runtime-claude");
  await expect(
    page.getByTestId("onboarding-runtime-install-claude"),
  ).toHaveText("SET UP");

  await claudeCard.focus();
  await claudeCard.press("Enter");
  await expect(
    page.getByTestId("onboarding-runtime-installed-claude"),
  ).toHaveText("INSTALLED");
});

test("config page shows Agent defaults form", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToConfigPage(page);

  // The defaults form is the page's content; no readiness badge is shown.
  await expect(page.locator("#global-agent-provider")).toBeVisible();
  await expect(page.locator("#global-agent-model")).toBeVisible();
  await expect(page.getByText("Default LLM provider")).toHaveCount(0);
  await expect(page.getByText("Select provider")).toHaveCount(0);
  await expect(page.getByTestId("global-agent-provider")).toHaveText(
    "Select a provider",
  );
  const modelSelect = page.getByTestId("global-agent-model");
  const effortSelect = page.getByTestId("global-agent-thinking-effort-select");
  await expect(modelSelect).toHaveText("Select a model");
  await expect(modelSelect).toBeDisabled();
  await expect(effortSelect).toHaveText("Select effort level");
  await expect(effortSelect).toBeDisabled();
  await page.getByTestId("global-agent-provider").click();
  await expect(
    page.getByTestId("global-agent-provider-option-__custom_provider__"),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByText("Applies to all agents")).toHaveCount(0);
  await expect(page.getByLabel("OpenAI API Key")).toHaveCount(0);
  await expect(effortSelect).toBeVisible();
  await expect(
    page.getByText("This will be set as your default model configuration"),
  ).toHaveCount(0);
  await expect(page.getByTestId("agent-readiness-badge")).toHaveCount(0);

  await waitForAnimations(page);
  const configPage = page.locator('[data-testid="onboarding-page-config"]');
  await configPage.screenshot({
    path: `${SHOTS}/04-config-defaults-form.png`,
  });
});

test("config page gates stale saved model and effort until provider selection", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      globalAgentConfig: {
        env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
        provider: null,
        model: "claude-sonnet-4-6",
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  const modelSelect = page.getByTestId("global-agent-model");
  const effortSelect = page.getByTestId("global-agent-thinking-effort-select");
  await expect(page.getByTestId("global-agent-provider")).toHaveText(
    "Select a provider",
  );
  await expect(modelSelect).toHaveText("Select a model");
  await expect(modelSelect).toBeDisabled();
  await expect(effortSelect).toHaveText("Select effort level");
  await expect(effortSelect).toBeDisabled();

  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");

  await expect(modelSelect).toBeEnabled();
  await expect(modelSelect).toHaveAttribute("data-value", "gpt-5.5");
  await expect(effortSelect).toBeEnabled();
  await expect(effortSelect).toHaveText("Select effort level");
});

test("config page defaults model after provider selection", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToConfigPage(page);

  const modelSelect = page.getByTestId("global-agent-model");
  const effortSelect = page.getByTestId("global-agent-thinking-effort-select");
  await expect(modelSelect).toHaveText("Select a model");
  await expect(modelSelect).toBeDisabled();
  await expect(effortSelect).toBeDisabled();

  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");

  await expect(page.getByTestId("global-agent-provider")).toHaveAttribute(
    "data-value",
    "openai",
  );
  await expect(modelSelect).toBeEnabled();
  await expect(modelSelect).toHaveAttribute("data-value", "gpt-5.5");
  await expect(modelSelect).toHaveText("gpt-5.5");
  await expect(effortSelect).toBeEnabled();
  await effortSelect.click();
  await expect(
    page.getByTestId("global-agent-thinking-effort-select-option-none"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-thinking-effort-select-option-minimal"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-thinking-effort-select-option-max"),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await modelSelect.click();
  await expect(
    page.getByTestId("global-agent-model-option-__custom_model__"),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
});

test("config page waits for baked defaults before showing provider dropdown", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      bakedBuildEnv: [
        { key: "BUZZ_AGENT_PROVIDER", masked: false, value: "anthropic" },
        { key: "BUZZ_AGENT_MODEL", masked: false, value: "claude-sonnet-4" },
      ],
      bakedBuildEnvDelayMs: 1_000,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  await expect(page.getByText("Loading…")).toBeVisible();
  const providerSelect = page.getByTestId("global-agent-provider");
  await expect(providerSelect).toHaveText("Anthropic");
  await expect(page.getByText("Select provider")).toHaveCount(0);
});

test("setup page shows provider discovery loading state", async ({ page }) => {
  await installMockBridge(
    page,
    { acpRuntimesDelayMs: 1_000 },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await page.getByRole("button", { name: "Get started" }).click();
  await passThroughBackupStep(page);

  await expect(page.getByTestId("onboarding-runtime-loading")).toBeVisible();
  await expect(page.getByText("Finding your providers...")).toBeVisible();

  await expect(page.getByTestId("onboarding-runtime-goose")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-loading")).toHaveCount(0);
});

test("config page stays compact when Buzz Agent model config is available", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("buzz-agent", { status: "not_applicable" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  // The compact onboarding form stays bare; readiness copy belongs in Settings.
  await expect(
    page.getByText("You can finish now and configure agents later in Settings"),
  ).toHaveCount(0);

  await waitForAnimations(page);
  const configPage = page.locator('[data-testid="onboarding-page-config"]');
  await configPage.screenshot({
    path: `${SHOTS}/05-config-compact.png`,
  });
});

test("Finish button is always enabled on config page regardless of readiness", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("buzz-agent", { status: "not_applicable" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  const finishBtn = page.getByTestId("onboarding-finish");
  await expect(finishBtn).toBeVisible();
  await expect(finishBtn).toBeEnabled();
});

test("community setup back button returns to agent defaults", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToConfigPage(page);
  await page.getByTestId("onboarding-finish").click();

  await expect(page.getByText("Join or create a community")).toBeVisible();

  await page.getByTestId("community-setup-back").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
});

// ---------------------------------------------------------------------------
// B1 regression: rapid consecutive edits must not lose the later change
// ---------------------------------------------------------------------------

test("Goose config page discovers models through the selected Goose runtime", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("goose", { status: "not_applicable" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await page.getByTestId("onboarding-runtime-goose").click();
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");
  await page.getByTestId("global-agent-model").click();
  await expect(
    page.getByTestId("global-agent-model-option-gpt-5.5"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
});

test("compact default config still persists rapid provider edits", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");
  await navigateToConfigPage(page);

  const providerSelect = page.getByTestId("global-agent-provider");
  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");
  await expect(providerSelect).toHaveAttribute("data-value", "openai");

  await chooseConfigDropdownOption(
    page,
    "global-agent-provider",
    "openai-compat",
  );
  await expect(providerSelect).toHaveAttribute("data-value", "openai-compat");

  await chooseConfigDropdownOption(page, "global-agent-provider", "anthropic");
  await expect(providerSelect).toHaveAttribute("data-value", "anthropic");

  await expect(page.getByLabel("Anthropic API Key")).toHaveCount(0);
  await expect(page.getByLabel("OpenAI API Key")).toHaveCount(0);
  await expect(page.getByLabel("Value for DATABRICKS_HOST")).toHaveCount(0);
});

test("rapid consecutive provider changes both survive — later change wins", async ({
  page,
}) => {
  // Hold each set_global_agent_config request for 300 ms so the test can
  // make a second edit before the first response arrives.
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("buzz-agent", { status: "not_applicable" }),
      ],
      setGlobalAgentConfigDelayMs: 300,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  const providerSelect = page.getByTestId("global-agent-provider");
  await expect(providerSelect).toBeVisible();

  // First edit: select OpenAI — save starts, held open for 300 ms.
  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");

  // Second edit before first response: select Anthropic. The coalescer must
  // persist this as the trailing save, and it must survive in the UI.
  await chooseConfigDropdownOption(page, "global-agent-provider", "anthropic");

  // Wait long enough for both saves to complete (2 × 300 ms + margin).
  await page.waitForTimeout(800);

  // The final provider shown must be Anthropic — neither save must overwrite
  // the later optimistic state with a stale response.
  await expect(providerSelect).toHaveAttribute("data-value", "anthropic");
});
