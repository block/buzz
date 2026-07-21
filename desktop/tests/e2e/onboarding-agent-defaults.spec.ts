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

test("defaults trusts setup readiness and persists the user's visible harness choice", async ({
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
