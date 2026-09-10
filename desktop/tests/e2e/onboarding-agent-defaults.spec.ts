import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { passThroughBackupStep } from "../helpers/onboarding";

function runtime(
  id: "buzz-agent" | "claude" | "codex" | "goose",
  availability: string,
  authStatus: Record<string, unknown>,
  overrides: Record<string, unknown> = {},
) {
  return {
    id,
    label:
      id === "buzz-agent"
        ? "Buzz Agent"
        : id === "claude"
          ? "Claude Code"
          : id === "codex"
            ? "Codex"
            : "Goose",
    avatar_url: "",
    availability,
    command: availability === "available" ? id : null,
    binary_path: availability === "available" ? `/usr/local/bin/${id}` : null,
    default_args: [],
    mcp_command: null,
    install_hint: `Install ${id}`,
    install_instructions_url: "https://example.com",
    can_auto_install: true,
    underlying_cli_path: null,
    node_required: false,
    auth_status: authStatus,
    login_hint: `Sign in to ${id}`,
    ...overrides,
  };
}

async function navigateToSetupPage(
  page: Parameters<typeof installMockBridge>[0],
  method: "subscription" | "api" | null = "subscription",
) {
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await page.getByRole("button", { name: "Create my private key" }).click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  if (method) {
    await page.getByTestId(`onboarding-harness-method-${method}`).click();
  }
}

async function chooseHarnessAndContinue(
  page: Parameters<typeof installMockBridge>[0],
  runtimeId = "claude",
) {
  if (await page.getByTestId("onboarding-page-config").isVisible()) return;
  await page.getByTestId(`onboarding-runtime-details-${runtimeId}`).click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
}

async function readSavedRuntime(page: Parameters<typeof installMockBridge>[0]) {
  return await page.evaluate(async () => {
    const result = await (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<{ preferred_runtime?: string | null }>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null);
    return result?.preferred_runtime ?? null;
  });
}

async function readGlobalConfigSetterCallCount(
  page: Parameters<typeof installMockBridge>[0],
) {
  return await page.evaluate(async () => {
    return await (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<number>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "get_global_agent_config_set_call_count",
      null,
    );
  });
}

test("setup filters the bundled harnesses by connection method", async ({
  page,
}) => {
  const renderErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") renderErrors.push(message.text());
  });
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
        runtime("goose", "available", { status: "not_applicable" }),
        runtime("codex", "available", { status: "logged_in" }),
        runtime("claude", "available", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page, null);

  await expect(
    page.getByRole("heading", { name: "Connect your AI provider" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("onboarding-harness-method-subscription"),
  ).toContainText("Log in with a subscription");
  await expect(page.getByTestId("onboarding-harness-method-api")).toContainText(
    "Use an API key",
  );
  await page.getByTestId("onboarding-harness-method-subscription").click();

  await expect(page.getByTestId("onboarding-runtime-claude")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-codex")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-goose")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-runtime-buzz-agent")).toHaveCount(
    0,
  );
  await page.getByTestId("onboarding-back").click();
  await page.getByTestId("onboarding-harness-method-api").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Connect with an API key" }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Choose your provider and enter an API key to connect to the Buzz harness.",
    ),
  ).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveCount(0);
  await expect(
    page.getByTestId("onboarding-use-different-harness"),
  ).toBeVisible();

  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Connect your AI provider" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose a harness" }),
  ).toHaveCount(0);

  await page.getByTestId("onboarding-harness-method-api").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await page.getByTestId("onboarding-use-different-harness").click();

  await expect(
    page.getByRole("heading", { name: "Choose a harness" }),
  ).toBeVisible();
  await page.waitForTimeout(250);
  await expect(
    page.getByRole("heading", { name: "Choose a harness" }),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-back")).toBeEnabled();
  await expect(
    renderErrors.filter((message) => message.includes("Maximum update depth")),
  ).toHaveLength(0);
  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Connect with an API key" }),
  ).toBeVisible();
  await expect(page.getByTestId("global-agent-provider")).toBeVisible();
  await page.getByTestId("onboarding-use-different-harness").click();
  await expect(
    page.getByRole("heading", { name: "Choose a harness" }),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-goose")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-buzz-agent")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-claude")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-runtime-codex")).toHaveCount(0);
  await expect(page.getByRole("checkbox")).toHaveCount(0);
  await expect(page.getByText(/More harnesses can be added in/)).toHaveCount(0);

  const recommended = page.getByTestId(
    "onboarding-runtime-recommended-buzz-agent",
  );
  const chevron = page.getByTestId("onboarding-runtime-chevron-buzz-agent");
  await expect(recommended).toBeVisible();
  await expect(
    page.getByTestId("onboarding-runtime-ready-buzz-agent"),
  ).toHaveCount(0);
  const [recommendedBox, chevronBox] = await Promise.all([
    recommended.boundingBox(),
    chevron.boundingBox(),
  ]);
  if (!recommendedBox || !chevronBox) {
    throw new Error("Could not measure harness status placement");
  }
  expect(recommendedBox.x).toBeLessThan(chevronBox.x);

  const [iconBox, titleBox] = await Promise.all([
    page.getByTestId("onboarding-runtime-icon-buzz-agent").boundingBox(),
    page.getByTestId("onboarding-runtime-title-buzz-agent").boundingBox(),
  ]);
  if (!iconBox || !titleBox) {
    throw new Error("Could not measure harness icon alignment");
  }
  const iconCenter = iconBox.y + iconBox.height / 2;
  const titleCenter = titleBox.y + titleBox.height / 2;
  expect(Math.abs(iconCenter - titleCenter)).toBeLessThanOrEqual(1);

  const selectableBuzzCard = page.getByTestId("onboarding-runtime-buzz-agent");
  await selectableBuzzCard.hover();
  await expect(selectableBuzzCard).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  const [rowBackground, recommendedBackground] = await Promise.all([
    selectableBuzzCard.evaluate(
      (element) => window.getComputedStyle(element).backgroundColor,
    ),
    recommended.evaluate(
      (element) => window.getComputedStyle(element).backgroundColor,
    ),
  ]);
  expect(recommendedBackground).not.toBe(rowBackground);

  const setupSkip = page.getByTestId("onboarding-setup-skip");
  await expect(setupSkip).toBeVisible();
  await expect(page.getByTestId("onboarding-setup-next")).toHaveCount(0);
  await expect(setupSkip).not.toHaveClass(/animate-in|fade-in/);

  await page.getByTestId("onboarding-runtime-details-buzz-agent").click();
  await expect(
    page.getByRole("heading", { name: "Connect with an API key" }),
  ).toBeVisible();
  await expect(page.getByTestId("global-agent-provider")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose your model settings" }),
  ).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next")).toHaveCount(0);
});

test("API selection survives a pending forced discovery with multiple ready harnesses", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
        runtime("goose", "available", { status: "not_applicable" }),
      ],
      acpRuntimesDelayMs: 3_000,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page, null);

  // Prime the shared catalog, then leave and re-enter setup so the second
  // forced discovery runs against cached multi-runtime readiness.
  await page.getByTestId("onboarding-harness-method-api").click();
  await expect(
    page.getByRole("heading", { name: "Choose a harness" }),
  ).toBeVisible();
  await page.getByTestId("onboarding-runtime-details-buzz-agent").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Choose a harness" }),
  ).toBeVisible();
  await page.getByTestId("onboarding-back").click();

  await page.getByTestId("onboarding-harness-method-api").click();
  await expect(page.getByTestId("onboarding-page-config")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible({
    timeout: 10_000,
  });
  await expect(page.getByTestId("global-agent-provider")).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveCount(0);
  expect(await readSavedRuntime(page)).toBeNull();
});

test("setup distinguishes a missing CLI from an installed desktop app", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime(
          "codex",
          "not_installed",
          { status: "unknown" },
          {
            install_hint: "Buzz talks to Codex through the Codex CLI.",
            install_instructions_url:
              "https://developers.openai.com/codex/cli/",
          },
        ),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const card = page.getByTestId("onboarding-runtime-codex");
  await expect(card).not.toContainText("CLI not detected");
  await expect(
    card.getByTestId("onboarding-runtime-install-codex"),
  ).toHaveCount(0);

  await page.getByTestId("onboarding-runtime-details-codex").click();
  await expect(
    page.getByTestId("onboarding-harness-setup-guide"),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Set up Codex" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("onboarding-harness-open-setup-guide"),
  ).toBeVisible();
  const setupGuideCard = page.getByTestId(
    "onboarding-harness-setup-guide-card",
  );
  await expect(setupGuideCard).toContainText("Codex");
  await expect(setupGuideCard).toContainText(
    "Codex is not detected on this computer.",
  );
  await expect(
    setupGuideCard.getByTestId("onboarding-harness-open-setup-guide"),
  ).toHaveText("Open guide");
  await expect(
    page.getByTestId("onboarding-runtime-install-codex"),
  ).toHaveCount(0);
  await expect(
    page
      .getByTestId("onboarding-page-2")
      .locator(".buzz-onboarding-transition-line"),
  ).toHaveAttribute("data-onboarding-direction", "forward");

  await page.getByTestId("onboarding-back").click();
  await expect(card).toBeVisible();
  await expect(
    page
      .getByTestId("onboarding-page-2")
      .locator(".buzz-onboarding-transition-line"),
  ).toHaveAttribute("data-onboarding-direction", "backward");
});

test("setup explains when an installed ACP adapter needs updating", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("codex", "adapter_outdated", { status: "unknown" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await page.getByTestId("onboarding-runtime-details-codex").click();
  await expect(
    page.getByTestId("onboarding-harness-setup-guide-card"),
  ).toContainText("Codex needs an ACP adapter update.");
});

test("a ready harness opens its provider settings without an intermediate page", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
        runtime("codex", "available", { status: "logged_out" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await expect(page.getByTestId("onboarding-runtime-ready-claude")).toHaveCount(
    0,
  );
  await expect(
    page.getByTestId("onboarding-runtime-checkmark-claude"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("onboarding-runtime-checkmark-codex"),
  ).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next")).toHaveCount(0);
  await page.getByTestId("onboarding-runtime-details-claude").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Choose your model settings" }),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-setup-next")).toHaveCount(0);
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  expect(await readSavedRuntime(page)).toBeNull();
});

test("setup shows runtime discovery loading before rendering harnesses", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      acpRuntimesDelayMs: 3_000,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await expect(page.getByTestId("onboarding-runtime-loading")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-loading")).toHaveText(
    "Loading providers…",
  );
  await expect(page.getByTestId("onboarding-runtime-claude")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-loading")).toHaveCount(0);
});

test("unknown authentication can be checked again", async ({ page }) => {
  const unknown = runtime("claude", "available", { status: "unknown" });
  const loggedIn = runtime("claude", "available", { status: "logged_in" });
  await installMockBridge(
    page,
    { acpRuntimesCatalogSequence: [[unknown], [loggedIn]] },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const checkAgain = page.getByRole("button", {
    name: "Check Claude Code again",
  });
  await expect(checkAgain).toHaveText("Check again");
  await checkAgain.click();
  await expect(page.getByTestId("onboarding-runtime-claude")).toHaveAttribute(
    "data-ready",
    "true",
  );
});

test("auth discovery failure stays actionable without exposing internals", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_out" }),
      ],
      acpAuthMethodsError: "sensitive auth discovery details",
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const signInRequired = page.getByTestId(
    "onboarding-runtime-sign-in-required-claude",
  );
  await expect(signInRequired).toHaveText("Sign in required");
  await expect(signInRequired).not.toHaveClass(/font-mono/);
  await page.getByTestId("onboarding-runtime-details-claude").click();
  await expect(
    page.getByRole("status", { name: /Sign-in unavailable/ }),
  ).toBeVisible();
  await expect(
    page.getByTestId("onboarding-runtime-instructions-claude"),
  ).toHaveText("Sign in");
  await expect(page.locator("body")).not.toContainText(
    "sensitive auth discovery details",
  );
});

test("terminal launch failure keeps Sign in available", async ({ page }) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_out" }),
      ],
      acpAuthMethods: {
        claude: {
          methods: [
            {
              id: "subscription",
              name: "Claude.ai subscription",
              description: null,
              type: "terminal",
            },
          ],
        },
      },
      connectAcpRuntimeError: "sensitive launch details",
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await expect(
    page.getByTestId("onboarding-runtime-sign-in-required-claude"),
  ).toHaveText("Sign in required");
  await page.getByTestId("onboarding-runtime-details-claude").click();
  const signIn = page.getByRole("button", { name: "Sign in to Claude Code" });
  await signIn.click();
  await expect(
    page.getByRole("status", { name: /Sign-in failed/ }),
  ).toBeVisible();
  await expect(signIn).toHaveText("Sign in");
  await expect(page.locator("body")).not.toContainText(
    "sensitive launch details",
  );
});

test("sign in stays pending until catalog detection confirms Ready", async ({
  page,
}) => {
  const loggedOut = runtime("claude", "available", { status: "logged_out" });
  const loggedIn = runtime("claude", "available", { status: "logged_in" });
  await installMockBridge(
    page,
    {
      acpRuntimesCatalogSequence: [[loggedOut], [loggedOut], [loggedIn]],
      acpAuthMethods: {
        claude: {
          methods: [
            {
              id: "subscription",
              name: "Claude.ai subscription",
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

  await page.getByTestId("onboarding-runtime-details-claude").click();
  await expect(
    page.getByText("Claude subscription", { exact: true }),
  ).toBeVisible();
  await expect(
    page.getByText("Buzz will open a sign-in window for Claude Code."),
  ).toBeVisible();
  const signIn = page.getByRole("button", { name: "Sign in to Claude Code" });
  await expect(signIn).toHaveText("Sign in");
  await expect(page.getByTestId("onboarding-setup-next")).toHaveCount(0);
  await signIn.click();
  await expect(signIn).toHaveText("Checking…");
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible({
    timeout: 5_000,
  });
  await expect(
    page.getByRole("heading", { name: "Choose your model settings" }),
  ).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
});

test("defaults waits for baked configuration before rendering fields", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      bakedBuildEnv: [
        { key: "ANTHROPIC_API_KEY", masked: true, value: "••••••" },
      ],
      bakedBuildEnvDelayMs: 500,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByText("Loading…")).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
});

test("defaults renders only fields supported by the selected harness", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      globalAgentConfig: {
        env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
        provider: null,
        model: "stale-model",
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  await expect(page.getByTestId("global-agent-provider")).toHaveCount(0);
  await expect(page.getByTestId("global-agent-model")).toHaveText(
    "Default model",
  );
  await expect(
    page.getByTestId("global-agent-thinking-effort-select"),
  ).toHaveCount(0);
});

test("defaults hides model when optional harness has empty discovery", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      discoverAgentModels: {
        models: [],
        supportsSwitching: false,
      },
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  // Confirmed successful empty catalog — omit the Model control; harness
  // default applies and Finish stays available.
  await expect(page.getByTestId("global-agent-model")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});

test("defaults keeps model control when optional harness discovery fails", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      discoverAgentModelsError: "CLI discovery timed out",
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  // Failed discovery must not look like successful empty: keep the control
  // and surface #2246 failure UI (status line bypasses onboarding-essential).
  await expect(page.getByTestId("global-agent-model")).toBeVisible();
  await expect(page.getByText(/Could not load live models/i)).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});

test("defaults can be skipped while loading without persisting configuration", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      bakedBuildEnvDelayMs: 500,
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByText("Loading…")).toBeVisible();
  await page.getByTestId("onboarding-config-skip").click();

  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBeNull();
});

test("defaults stages auto-selection and edits without writing when skipped", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  await page.getByTestId("global-agent-model").click();
  await page
    .getByTestId("global-agent-model-option-claude-opus-4-20250514")
    .click();
  expect(await readGlobalConfigSetterCallCount(page)).toBe(0);
  await expect(
    page.getByText(
      "Configure default models in Settings → Agents after setup.",
    ),
  ).toHaveCount(0);

  await page.getByTestId("onboarding-config-skip").click();

  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBeNull();
  expect(await readGlobalConfigSetterCallCount(page)).toBe(0);
});

test("Back preserves incomplete defaults draft without writing", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
        runtime("claude", "available", { status: "logged_in" }),
      ],
      discoverAgentModels: {
        models: [{ id: "claude-sonnet-4", name: "Claude Sonnet 4" }],
        supportsSwitching: true,
      },
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page, "api");
  await chooseHarnessAndContinue(page);
  await expect(
    page
      .getByTestId("onboarding-page-config")
      .locator(".buzz-onboarding-transition-line"),
  ).toHaveAttribute("data-onboarding-direction", "forward");

  await expect(page.getByTestId("global-agent-default-harness")).toHaveCount(0);
  await page.getByTestId("global-agent-provider").click();
  await page.getByTestId("global-agent-provider-option-anthropic").click();
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();

  await page.getByTestId("onboarding-back").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  await expect(
    page
      .getByTestId("onboarding-page-2")
      .locator(".buzz-onboarding-transition-line"),
  ).toHaveAttribute("data-onboarding-direction", "backward");
  expect(await readSavedRuntime(page)).toBeNull();
  expect(await readGlobalConfigSetterCallCount(page)).toBe(0);

  await page.getByTestId("onboarding-harness-method-api").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(
    page
      .getByTestId("onboarding-page-config")
      .locator(".buzz-onboarding-transition-line"),
  ).toHaveAttribute("data-onboarding-direction", "forward");
  await expect(page.getByTestId("global-agent-default-harness")).toHaveCount(0);
  await expect(page.getByTestId("global-agent-provider")).toHaveText(
    "Anthropic",
  );
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();
  expect(await readGlobalConfigSetterCallCount(page)).toBe(0);
});

test("defaults auto-selects the only ready visible harness", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "not_installed", { status: "not_applicable" }),
        runtime("goose", "not_installed", { status: "not_applicable" }),
        runtime("claude", "available", { status: "logged_in" }),
        runtime("codex", "available", { status: "logged_out" }),
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
  expect(await readSavedRuntime(page)).toBeNull();
});

test("Next persists the harness chosen from the subscription list", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
        runtime("codex", "available", { status: "logged_in" }),
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
      setGlobalAgentConfigDelayMs: 300,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  const finish = page.getByTestId("onboarding-finish");
  await expect(finish).toBeEnabled();
  expect(await readGlobalConfigSetterCallCount(page)).toBe(0);
  await finish.click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  await expect.poll(() => readSavedRuntime(page)).toBe("claude");
});

test("Next shows saving state and advances only after persistence", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
        runtime("codex", "available", { status: "logged_in" }),
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
      setGlobalAgentConfigDelayMs: 500,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);

  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  await page.getByTestId("onboarding-finish").click();

  await expect(page.getByTestId("onboarding-finish")).toHaveText("Saving…");
  await expect(page.getByTestId("onboarding-config-skip")).toBeDisabled();
  await expect(page.getByTestId("onboarding-back")).toBeDisabled();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  expect(await readSavedRuntime(page)).toBeNull();

  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("claude");
});

test("Next keeps the draft and retries after a save failure", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
      setGlobalAgentConfigErrors: ["Disk is read-only", null],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );

  await page.getByTestId("onboarding-finish").click();

  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(page.getByTestId("onboarding-config-save-error")).toContainText(
    "Disk is read-only",
  );
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
  expect(await readSavedRuntime(page)).toBeNull();
  expect(await readGlobalConfigSetterCallCount(page)).toBe(1);

  await page.getByTestId("onboarding-finish").click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("claude");
  expect(await readGlobalConfigSetterCallCount(page)).toBe(2);
});

test("defaults carries the chosen subscription harness forward", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
        runtime("goose", "available", { status: "not_applicable" }),
        runtime("claude", "available", { status: "logged_in" }),
        runtime("codex", "available", { status: "logged_in" }),
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await chooseHarnessAndContinue(page);
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  const harness = page.getByTestId("global-agent-default-harness");
  await expect(harness).toHaveText("Claude Code");
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
  await harness.click();
  await expect(
    page.getByTestId("global-agent-default-harness-option-claude"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-default-harness-option-codex"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-default-harness-option-goose"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-default-harness-option-buzz-agent"),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  expect(await readSavedRuntime(page)).toBeNull();
});

/**
 * Two installs started concurrently — claude fails with a multiline error
 * (rich hint+stderr in the tooltip) while codex succeeds. Each card must
 * keep its own independent spinner and its own terminal result; neither card
 * may show the other's outcome.
 *
 * This is the behavioral regression test for the per-card mutation fix
 * (Bug B) and the multiline tooltip fix (Bug A / F3 from Thufir pass 1).
 */
test("Finish stays disabled until a provider-required harness is fully configured", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
      ],
      discoverAgentModels: {
        models: [{ id: "claude-sonnet-4", name: "Claude Sonnet 4" }],
        supportsSwitching: true,
      },
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page, "api");
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  // buzz-agent auto-selects as the only ready harness, but with no provider
  // configured the default is not launchable — Finish must be gated.
  await expect(page.getByTestId("global-agent-default-harness")).toHaveCount(0);
  const finish = page.getByTestId("onboarding-finish");
  await expect(finish).toBeDisabled();

  // Configure provider + credential; model resolves via discovery/fallback.
  await page.getByTestId("global-agent-provider").click();
  await page.getByTestId("global-agent-provider-option-anthropic").click();
  await expect(
    page.getByText("ANTHROPIC_API_KEY", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByTestId("global-agent-model")).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-thinking-effort-select"),
  ).toHaveCount(0);

  await page.getByTestId("persona-provider-api-key").fill("sk-test-key");
  await expect(page.getByTestId("global-agent-model")).toBeVisible();
  await expect(
    page.getByTestId("global-agent-thinking-effort-select"),
  ).toBeVisible();

  const modelBox = await page.getByTestId("global-agent-model").boundingBox();
  const effortBox = await page
    .getByTestId("global-agent-thinking-effort-select")
    .boundingBox();
  expect(modelBox).not.toBeNull();
  expect(effortBox).not.toBeNull();
  expect(Math.abs((modelBox?.y ?? 0) - (effortBox?.y ?? 0))).toBeLessThan(2);
  expect(modelBox?.x ?? 0).toBeLessThan(effortBox?.x ?? 0);

  await expect(finish).toBeEnabled();
  await finish.click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("buzz-agent");
});

test("API key options stay hidden when credential validation is not accepted", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
      ],
      discoverAgentModelsError:
        "Anthropic model discovery HTTP 401: invalid x-api-key",
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page, "api");
  await page.getByTestId("global-agent-provider").click();
  await page.getByTestId("global-agent-provider-option-anthropic").click();
  await page.getByTestId("persona-provider-api-key").fill("invalid-key");

  await expect(
    page.getByText(
      "We couldn’t validate this API key. Check the key or your connection and try again.",
    ),
  ).toBeVisible();
  await expect(page.getByTestId("global-agent-model")).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-thinking-effort-select"),
  ).toHaveCount(0);
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();
});

test("baked build config keeps Finish enabled without manual provider setup", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
      ],
      bakedBuildEnv: [
        { key: "BUZZ_AGENT_PROVIDER", masked: false, value: "databricks_v2" },
        {
          key: "DATABRICKS_HOST",
          masked: false,
          value: "https://example.cloud.databricks.com",
        },
        { key: "DATABRICKS_MODEL", masked: false, value: "baked-model" },
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
        preferred_runtime: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page, "api");
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  // Internal builds bake provider/model/credentials — the gate must treat
  // baked config as complete and never block Finish.
  await expect(page.getByTestId("global-agent-default-harness")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});
