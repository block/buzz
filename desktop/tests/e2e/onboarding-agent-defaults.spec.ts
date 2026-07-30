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

async function navigateToChooserPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await passThroughBackupStep(page);
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
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

test("chooser shows all bundled harnesses without setup controls", async ({
  page,
}) => {
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
  await navigateToChooserPage(page);

  await expect(page.getByText("Choose your default harness")).toBeVisible();
  await expect(
    page.getByText(/connect and add more harnesses later/i),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-claude")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-codex")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-goose")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-buzz-agent")).toBeVisible();
  await expect(page.getByText("READY")).toHaveCount(0);
  await expect(page.getByRole("button", { name: /install/i })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /sign in/i })).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next")).toHaveCount(0);
  expect(await readSavedRuntime(page)).toBeNull();
});

test("Skip for now completes onboarding without saving a default", async ({
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
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);

  await page.getByTestId("onboarding-setup-skip").click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBeNull();
});

test("card click is local, advances to setup, and Finish persists the default", async ({
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
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);

  const chooserCard = page.getByTestId("onboarding-runtime-claude");
  const chooserBox = await chooserCard.boundingBox();
  await chooserCard.click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(page.getByText("Set up Claude Code")).toBeVisible();
  const setupCard = page.getByTestId("onboarding-selected-runtime-card");
  const setupBox = await setupCard.boundingBox();
  expect(chooserBox).not.toBeNull();
  expect(setupBox).not.toBeNull();
  expect(Math.round(setupBox?.width ?? 0)).toBe(
    Math.round(chooserBox?.width ?? 0),
  );
  expect(Math.round(setupBox?.height ?? 0)).toBe(
    Math.round(chooserBox?.height ?? 0),
  );
  expect(await readSavedRuntime(page)).toBeNull();

  await page.getByTestId("onboarding-finish").click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("claude");
});

test("Back returns to chooser with the pick remembered", async ({ page }) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
        runtime("codex", "available", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);

  await page.getByTestId("onboarding-runtime-codex").click();
  // Card click fades the unselected cards before advancing — wait for the
  // setup page so Back targets it rather than the chooser.
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await page.getByTestId("onboarding-back").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-codex")).toHaveAttribute(
    "data-selected",
    "true",
  );
});

test("Back returns to choose a different harness; last click wins", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
        runtime("codex", "available", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);

  await page.getByTestId("onboarding-runtime-claude").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await page.getByTestId("onboarding-back").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  await page.getByTestId("onboarding-runtime-codex").click();
  await page.getByTestId("onboarding-finish").click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("codex");
});

test("selected setup handles install before Finish", async ({ page }) => {
  const notInstalled = runtime("claude", "adapter_missing", {
    status: "unknown",
  });
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [notInstalled],
      installAcpRuntimeResults: [
        {
          success: false,
          steps: [
            {
              step: "adapter",
              command: "mock install claude",
              success: false,
              stdout: "",
              stderr: "sensitive install details",
              exit_code: 1,
            },
          ],
        },
        {
          success: true,
          steps: [
            {
              step: "adapter",
              command: "mock install claude",
              success: true,
              stdout: "installed",
              stderr: "",
              exit_code: 0,
            },
          ],
        },
      ],
      acpRuntimesCatalogAfterInstall: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  const install = page.getByTestId("onboarding-runtime-install-claude");
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();
  await install.click();
  const installError = page.getByTestId("onboarding-runtime-error-claude");
  await expect(installError).toBeVisible();
  await expect(installError).not.toContainText("sensitive install details");
  await expect(install).toHaveText("Retry install");
  await install.click();
  await expect(page.getByText(/Claude Code is ready/i)).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});

test("post-install catalog failure stays retryable without exposing internals", async ({
  page,
}) => {
  const notInstalled = runtime("claude", "adapter_missing", {
    status: "unknown",
  });
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [notInstalled],
      acpRuntimesErrors: [
        null,
        ...Array.from({ length: 4 }, () => "sensitive catalog details"),
      ],
      installAcpRuntimeResult: {
        success: true,
        steps: [
          {
            step: "adapter",
            command: "mock install claude",
            success: true,
            stdout: "installed",
            stderr: "",
            exit_code: 0,
          },
        ],
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();
  await page.getByTestId("onboarding-runtime-install-claude").click();

  await expect(
    page.getByText("Couldn't load harness settings. Try again."),
  ).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
  await expect(page.getByTestId("onboarding-page-config")).not.toContainText(
    "sensitive catalog details",
  );
});

test("selected setup handles sign-in before Finish", async ({ page }) => {
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  const signIn = page.getByRole("button", { name: "Sign in to Claude Code" });
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();
  await signIn.click();
  await expect(signIn).toHaveText("Checking…");
  await expect(page.getByText(/Claude Code is ready/i)).toBeVisible({
    timeout: 5_000,
  });
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});

test("Claude preserves API-key auth methods from main", async ({ page }) => {
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
              id: "api-key",
              name: "API key",
              description: null,
              type: "terminal",
            },
          ],
        },
      },
      connectAcpRuntimeResult: { launched: true },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  const signIn = page.getByRole("button", { name: "Sign in to Claude Code" });
  await expect(signIn).toBeEnabled();
  await signIn.click();
  await expect(signIn).toHaveText("Checking…");
});

test("sign-in failures stay actionable without exposing internals", async ({
  page,
}) => {
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  const signIn = page.getByRole("button", { name: "Sign in to Claude Code" });
  await signIn.click();
  await expect(
    page.getByRole("status", { name: /Sign-in failed/ }),
  ).toBeVisible();
  await expect(signIn).toHaveText("Sign in");
  await expect(page.getByTestId("onboarding-page-config")).not.toContainText(
    "sensitive launch details",
  );
});

test("auth discovery failures stay actionable without exposing internals", async ({
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  await expect(
    page.getByRole("status", { name: /Sign-in unavailable/ }),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-page-config")).not.toContainText(
    "sensitive auth discovery details",
  );
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  const checkAgain = page.getByRole("button", { name: "Check again" });
  await checkAgain.click();
  await expect(page.getByText(/Claude Code is ready/i)).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});

test("Goose provider setup omits the incompatible Buzz effort control", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("goose", "available", { status: "not_applicable" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-goose").click();

  await expect(page.getByTestId("global-agent-provider")).toBeVisible();
  await expect(page.getByTestId("global-agent-model")).toBeVisible();
  await expect(
    page.getByTestId("global-agent-thinking-effort-select"),
  ).toHaveCount(0);
});

test("Buzz baked defaults do not mark Goose ready", async ({ page }) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("goose", "available", { status: "not_applicable" }),
      ],
      bakedBuildEnv: [
        { key: "BUZZ_AGENT_PROVIDER", masked: false, value: "anthropic" },
        { key: "BUZZ_AGENT_MODEL", masked: false, value: "claude-sonnet-4" },
        { key: "ANTHROPIC_API_KEY", masked: true, value: "••••••" },
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-goose").click();

  await expect(page.getByTestId("global-agent-provider")).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();
  await expect(page.getByText(/Goose is ready/i)).toHaveCount(0);
});

test("provider setup is skipped when runtime file config already satisfies Goose", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("goose", "available", { status: "not_applicable" }),
      ],
      runtimeFileConfigs: {
        goose: {
          provider: "anthropic",
          model: "claude-sonnet-4",
          satisfiedEnvKeys: ["ANTHROPIC_API_KEY"],
        },
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-goose").click();

  await expect(page.getByText(/Goose is ready/i)).toBeVisible();
  await expect(page.getByTestId("global-agent-provider")).toHaveCount(0);
  await expect(page.getByTestId("global-agent-model")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});

test("Buzz Agent provider setup collects defaults before Finish", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
      ],
      discoverAgentModels: {
        models: [
          {
            id: "claude-sonnet-4",
            name: "Claude Sonnet 4",
          },
        ],
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-buzz-agent").click();

  await expect(page.getByTestId("global-agent-provider")).toBeVisible();
  await expect(page.getByTestId("global-agent-model")).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();

  const provider = page.getByTestId("global-agent-provider");
  await provider.click();
  await page.getByTestId("global-agent-provider-option-anthropic").click();
  await expect(page.getByTestId("global-agent-model")).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();

  const model = page.getByTestId("global-agent-model");
  await model.click();
  await page.getByTestId("global-agent-model-option-claude-sonnet-4").click();
  await page.getByTestId("persona-provider-api-key").fill("sk-ant-test");
  await expect(page.getByTestId("global-agent-provider")).toBeVisible();
  await expect(page.getByTestId("global-agent-model")).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
  await page.getByTestId("onboarding-finish").click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("buzz-agent");
});

test("baked build env satisfies Buzz Agent setup with no manual config", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
      ],
      bakedBuildEnv: [
        { key: "BUZZ_AGENT_PROVIDER", masked: false, value: "anthropic" },
        { key: "BUZZ_AGENT_MODEL", masked: false, value: "claude-sonnet-4" },
        { key: "ANTHROPIC_API_KEY", masked: true, value: "••••••" },
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-buzz-agent").click();

  // Internal builds bake provider/model/credential env — setup is skipped
  // entirely and Finish is immediately available.
  await expect(page.getByText(/Buzz is ready/i)).toBeVisible();
  await expect(page.getByTestId("global-agent-provider")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
  await page.getByTestId("onboarding-finish").click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("buzz-agent");
});

test("Finish preserves existing values when the selected harness is unchanged", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
      ],
      globalAgentConfig: {
        env_vars: {
          ANTHROPIC_API_KEY: "sk-existing",
          BUZZ_AGENT_THINKING_EFFORT: "high",
        },
        provider: "anthropic",
        model: "claude-sonnet-4",
        preferred_runtime: "buzz-agent",
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-buzz-agent").click();
  await page.getByTestId("onboarding-finish").click();
  await expect(page.getByText("Join or create a community")).toBeVisible();

  const saved = await page.evaluate(async () => {
    return await (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<{
          env_vars: Record<string, string>;
          model: string | null;
          provider: string | null;
        }>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null);
  });
  expect(saved).toMatchObject({
    env_vars: {
      ANTHROPIC_API_KEY: "sk-existing",
      BUZZ_AGENT_THINKING_EFFORT: "high",
    },
    model: "claude-sonnet-4",
    provider: "anthropic",
  });
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  await expect(page.getByTestId("global-agent-model")).toHaveCount(0);
  await expect(page.getByText(/Claude Code is ready/i)).toBeVisible();
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
  await navigateToChooserPage(page);
  await page.getByTestId("onboarding-runtime-claude").click();

  await expect(page.getByText(/Claude Code is ready/i)).toBeVisible();
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
});
