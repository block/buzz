import { expect, test } from "@playwright/test";
import { ACP_RUNTIME_VISIBILITY_STORAGE_KEY } from "@/features/agents/lib/runtimeVisibilityPreference";
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

test("setup shows only Claude Code and Codex as detected harnesses", async ({
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
  await navigateToSetupPage(page);

  await expect(page.getByTestId("onboarding-runtime-claude")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-codex")).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-goose")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-runtime-buzz-agent")).toHaveCount(
    0,
  );
  await expect(page.getByRole("checkbox")).toHaveCount(0);
});

test("ready state is detected and enables Next without persisting a default", async ({
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

  await expect(page.getByTestId("onboarding-runtime-ready-claude")).toHaveText(
    "READY",
  );
  await expect(
    page.getByTestId("onboarding-runtime-checkmark-claude"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("onboarding-runtime-checkmark-codex"),
  ).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
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
      acpRuntimesDelayMs: 500,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await expect(page.getByTestId("onboarding-runtime-loading")).toBeVisible();
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
  await expect(checkAgain).toHaveText("CHECK AGAIN");
  await checkAgain.click();
  await expect(page.getByTestId("onboarding-runtime-ready-claude")).toHaveText(
    "READY",
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

  const card = page.getByTestId("onboarding-runtime-claude");
  await expect(
    card.getByRole("status", { name: /Sign-in unavailable/ }),
  ).toBeVisible();
  await expect(
    card.getByTestId("onboarding-runtime-instructions-claude"),
  ).toHaveText("SIGN IN");
  await expect(card).not.toContainText("sensitive auth discovery details");
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

  const card = page.getByTestId("onboarding-runtime-claude");
  const signIn = card.getByRole("button", { name: "Sign in to Claude Code" });
  await signIn.click();
  await expect(
    card.getByRole("status", { name: /Sign-in failed/ }),
  ).toBeVisible();
  await expect(signIn).toHaveText("SIGN IN");
  await expect(card).not.toContainText("sensitive launch details");
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

  const signIn = page.getByRole("button", { name: "Sign in to Claude Code" });
  await expect(signIn).toHaveText("SIGN IN");
  await expect(page.getByTestId("onboarding-setup-next")).toBeDisabled();
  await signIn.click();
  await expect(signIn).toHaveText("CHECKING…");
  await expect(page.getByTestId("onboarding-setup-next")).toBeDisabled();
  await expect(page.getByTestId("onboarding-runtime-ready-claude")).toHaveText(
    "READY",
    { timeout: 5_000 },
  );
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
});

test("failed install can be retried without shifting card content", async ({
  page,
}) => {
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
  await navigateToSetupPage(page);

  const card = page.getByTestId("onboarding-runtime-claude");
  const heading = card.getByRole("heading", { name: "Claude Code" });
  const headingTop = await heading.evaluate(
    (element) => element.getBoundingClientRect().top,
  );
  const install = page.getByTestId("onboarding-runtime-install-claude");
  await install.click();
  const error = page.getByTestId("onboarding-runtime-error-claude");
  await expect(error).toBeVisible();
  await expect(install).toHaveText("RETRY INSTALL");
  await expect(error).not.toContainText("sensitive install details");
  expect(
    await heading.evaluate((element) => element.getBoundingClientRect().top),
  ).toBe(headingTop);
  await install.click();
  await expect(page.getByTestId("onboarding-runtime-ready-claude")).toHaveText(
    "READY",
  );
});

test("install transitions through Sign in to Ready", async ({ page }) => {
  const notInstalled = runtime("claude", "adapter_missing", {
    status: "unknown",
  });
  const loggedOut = runtime("claude", "available", { status: "logged_out" });
  const loggedIn = runtime("claude", "available", { status: "logged_in" });
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [notInstalled],
      acpRuntimesCatalogAfterInstallSequence: [[loggedOut], [loggedIn]],
      installAcpRuntimeDelayMs: 500,
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

  const install = page.getByTestId("onboarding-runtime-install-claude");
  await expect(install).toHaveText("INSTALL");
  await install.click();

  const signIn = page.getByRole("button", { name: "Sign in to Claude Code" });
  await expect(signIn).toHaveText("SIGN IN");
  await expect(page.getByTestId("onboarding-setup-next")).toBeDisabled();
  await signIn.click();
  await expect(page.getByTestId("onboarding-runtime-ready-claude")).toHaveText(
    "READY",
    { timeout: 5_000 },
  );
  await expect(
    page.getByTestId("onboarding-runtime-checkmark-claude"),
  ).toHaveCount(0);
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
  await page.getByTestId("onboarding-setup-next").click();

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
  await page.getByTestId("onboarding-setup-next").click();

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
  await page.getByTestId("onboarding-setup-next").click();

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
  await page.getByTestId("onboarding-setup-next").click();

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

test("defaults Back returns to harness setup", async ({ page }) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("claude", "available", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await page.getByTestId("onboarding-setup-next").click();
  await page.getByTestId("onboarding-back").click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
});

test("defaults auto-selects the only ready visible harness", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        runtime("buzz-agent", "available", { status: "not_applicable" }),
        runtime("goose", "available", { status: "not_applicable" }),
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
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude Code",
  );
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
  await expect.poll(() => readSavedRuntime(page)).toBe("claude");
});

test("defaults exclude device-disabled ready harnesses", async ({ page }) => {
  await page.addInitScript((storageKey) => {
    window.localStorage.setItem(storageKey, JSON.stringify(["claude"]));
  }, ACP_RUNTIME_VISIBILITY_STORAGE_KEY);
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
  await navigateToSetupPage(page);
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  const harness = page.getByTestId("global-agent-default-harness");
  await expect(harness).toHaveText("Codex");
  await harness.click();
  await expect(
    page.getByTestId("global-agent-default-harness-option-claude"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-default-harness-option-codex"),
  ).toBeVisible();
});

test("Finish waits for the latest rapid harness choice to persist", async ({
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
  await page.getByTestId("onboarding-setup-next").click();

  const harness = page.getByTestId("global-agent-default-harness");
  await harness.click();
  await page.getByTestId("global-agent-default-harness-option-claude").click();
  await harness.click();
  await page.getByTestId("global-agent-default-harness-option-codex").click();
  const finish = page.getByTestId("onboarding-finish");
  await expect(finish).toBeDisabled();
  await expect(finish).toBeEnabled({ timeout: 2_000 });
  await finish.click();
  await expect(page.getByText("Join or create a community")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("codex");
});

test("defaults requires a choice when multiple visible harnesses are ready", async ({
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
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  const harness = page.getByTestId("global-agent-default-harness");
  await expect(harness).toHaveText("Select a harness");
  await expect(page.getByTestId("onboarding-finish")).toBeDisabled();
  await harness.click();
  await expect(
    page.getByTestId("global-agent-default-harness-option-claude"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-default-harness-option-codex"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-default-harness-option-goose"),
  ).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-default-harness-option-buzz-agent"),
  ).toHaveCount(0);
  await page.getByTestId("global-agent-default-harness-option-codex").click();
  await expect(harness).toHaveText("Codex");
  await expect(page.getByTestId("onboarding-finish")).toBeEnabled();
  await expect.poll(() => readSavedRuntime(page)).toBe("codex");
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
test("concurrent installs each keep their own state — one fails, one succeeds", async ({
  page,
}) => {
  // Realistic 512-head + 1024-tail shape: many short lines followed by one
  // long unbroken Windows path.  This exercises both overflow axes:
  //   • vertical: enough lines to exceed max-h-48 (192px at ~16px/line)
  //   • horizontal: the long path has no spaces, so only break-words prevents
  //     scrollWidth > clientWidth.
  const longWindowsPath =
    "C:\\Users\\willp\\AppData\\Roaming\\npm\\node_modules\\@agentclientprotocol\\claude-agent-acp\\dist\\bin\\claude-agent-acp.exe";
  const multilineError = [
    "npm ERR! code EACCES",
    "npm ERR! syscall mkdir",
    "npm ERR! path C:\\Users\\willp\\AppData\\Roaming\\npm",
    "npm ERR! errno -4048",
    "npm ERR! Error: EACCES: permission denied, mkdir 'C:\\Users\\willp\\AppData\\Roaming\\npm'",
    "npm ERR!  { [Error: EACCES: permission denied, mkdir 'C:\\Users\\willp\\AppData\\Roaming\\npm']",
    "npm ERR!   errno: -4048,",
    "npm ERR!   code: 'EACCES',",
    "npm ERR!   syscall: 'mkdir',",
    "npm ERR!   path: 'C:\\\\Users\\\\willp\\\\AppData\\\\Roaming\\\\npm' }",
    "npm ERR!",
    "npm ERR! The operation was rejected by your operating system.",
    "npm ERR! It is likely you do not have the permissions to access this file as the current user",
    "npm ERR!",
    `npm ERR! If you believe this might be a permissions issue, please double-check the`,
    `npm ERR! permissions of the file and its containing directories, or try running`,
    `npm ERR! the command again as root/Administrator.`,
    "",
    `Hint: Run as Administrator or change npm prefix: npm config set prefix ${longWindowsPath}`,
  ].join("\n");
  const claudeNotInstalled = runtime("claude", "adapter_missing", {
    status: "unknown",
  });
  const codexNotInstalled = runtime("codex", "adapter_missing", {
    status: "unknown",
  });
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [claudeNotInstalled, codexNotInstalled],
      // Claude: long delay then failure with multiline stderr + hint.
      // Codex: short delay then success.
      // Per-runtime config lets both be in flight simultaneously.
      installAcpRuntimeByRuntime: {
        claude: {
          delayMs: 600,
          result: {
            success: false,
            steps: [
              {
                step: "adapter",
                command: "npm install -g @agentclientprotocol/claude-agent-acp",
                success: false,
                stdout: "",
                stderr: multilineError,
                exit_code: 1,
              },
            ],
          },
        },
        codex: {
          delayMs: 200,
          result: {
            success: true,
            steps: [
              {
                step: "adapter",
                command: "npm install -g @zed-industries/codex-acp",
                success: true,
                stdout: "added 1 package",
                stderr: "",
                exit_code: 0,
              },
            ],
          },
        },
      },
      acpRuntimesCatalogAfterInstall: [
        runtime("claude", "adapter_missing", { status: "unknown" }),
        runtime("codex", "available", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const claudeInstall = page.getByTestId("onboarding-runtime-install-claude");
  const codexInstall = page.getByTestId("onboarding-runtime-install-codex");

  // Start both installs before either settles.
  await claudeInstall.click();
  await codexInstall.click();

  // While in flight: both install buttons must be absent (no duplicate clicks).
  await expect(claudeInstall).toHaveCount(0);
  await expect(codexInstall).toHaveCount(0);

  // Codex settles first (shorter delay): success indicator, no error.
  await expect(page.getByTestId("onboarding-runtime-ready-codex")).toBeVisible({
    timeout: 3_000,
  });
  await expect(page.getByTestId("onboarding-runtime-error-codex")).toHaveCount(
    0,
  );

  // Claude still in flight: its install button must still be absent.
  await expect(claudeInstall).toHaveCount(0);

  // Claude settles: failure error visible; codex still shows ready (not reset).
  const claudeError = page.getByTestId("onboarding-runtime-error-claude");
  await expect(claudeError).toBeVisible({ timeout: 3_000 });
  await expect(
    page.getByTestId("onboarding-runtime-ready-codex"),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-runtime-error-codex")).toHaveCount(
    0,
  );

  // The error trigger has the full aria-label (label + detail).
  await expect(claudeError).toHaveAttribute("aria-label", /npm ERR!/);
  // Open the tooltip and verify the detail span handles overflow correctly:
  //   • vertical overflow exists and is scrollable (max-h-48 + overflow-y-auto)
  //   • no horizontal overflow (break-words forces the long unbroken path to wrap)
  await claudeError.focus();
  const tooltip = page.getByRole("tooltip");
  await expect(tooltip).toBeVisible({ timeout: 2_000 });
  await expect(tooltip).toContainText("npm ERR! code EACCES");
  await expect(tooltip).toContainText("Hint: Run as Administrator");

  // Locate the scroll container using page-level locator since Radix portals
  // can place content outside the tooltip role element's subtree in the DOM.
  // Use .first() because Radix keeps a hidden duplicate in the light DOM.
  const detailSpan = page.locator("span.overflow-y-auto").first();
  await expect(detailSpan).toBeVisible();

  // Vertical: scrollHeight must exceed clientHeight (content taller than max-h-48).
  // Scroll position must advance when set, proving scrollability.
  const isVerticallyScrollable = await detailSpan.evaluate((el) => {
    return el.scrollHeight > el.clientHeight;
  });
  expect(isVerticallyScrollable).toBe(true);

  // Confirm scroll position can actually advance.
  await detailSpan.evaluate((el) => {
    el.scrollTop = 9999;
  });
  const scrolledDown = await detailSpan.evaluate((el) => el.scrollTop > 0);
  expect(scrolledDown).toBe(true);

  // Horizontal: break-words must prevent horizontal overflow.
  const hasHorizontalOverflow = await detailSpan.evaluate((el) => {
    return el.scrollWidth > el.clientWidth;
  });
  expect(hasHorizontalOverflow).toBe(false);
});
