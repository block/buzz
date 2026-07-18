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
) {
  return {
    id,
    label: id === "buzz-agent" ? "Buzz Agent" : id,
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
  };
}

/** Drive to the harness setup page (page 3) via the full onboarding flow. */
async function navigateToSetupPage(
  page: Parameters<typeof installMockBridge>[0],
) {
  await page.getByRole("button", { name: "Create a new identity key" }).click();
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

test("requires a preferred runtime and routes detailed runtimes to config", async ({
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
  await expect(next).toBeDisabled();
  await page.getByTestId("onboarding-runtime-buzz-agent").click();
  await expect(next).toHaveText("Saving…");
  await expect(next).toBeDisabled();
  await expect(next).toBeEnabled();
  await next.click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
});

test("authenticated Claude persists as preferred and skips detailed config", async ({
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

  const savedConfig = await page.evaluate(() =>
    (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<{ preferred_runtime: string | null }>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null),
  );
  expect(savedConfig?.preferred_runtime).toBe("claude");
});

test("successful Claude sign-in selects it as preferred", async ({ page }) => {
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
  await expect(card.getByText("Preferred")).toBeVisible();
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();

  const savedConfig = await page.evaluate(() =>
    (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<{ preferred_runtime: string | null }>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null),
  );
  expect(savedConfig?.preferred_runtime).toBe("claude");
});

test("successful Codex sign-in hides API key auth and selects it as preferred", async ({
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
  await expect(card.getByLabel("Codex available")).toHaveCount(0);
  await expect(
    card.getByRole("button", { name: "Sign in with ChatGPT" }),
  ).toHaveCount(0);
  await card.getByRole("button", { name: "Log in" }).click({ force: true });
  await expect(card.getByText("Preferred")).toBeVisible();
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();

  const savedConfig = await page.evaluate(() =>
    (
      window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload: unknown,
        ) => Promise<{ preferred_runtime: string | null }>;
      }
    ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_global_agent_config", null),
  );
  expect(savedConfig?.preferred_runtime).toBe("codex");
});

test("runtime checkmarks only show for configured harnesses", async ({
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

  await expect(page.getByLabel("Claude available")).toHaveCount(0);
  await expect(page.getByLabel("codex available")).toBeVisible();
});

test("runtime cards use the preferred onboarding order", async ({ page }) => {
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
    .getByRole("radio")
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

for (const authStatus of [
  { status: "logged_out" as const },
  { status: "unknown" as const },
  { status: "config_invalid" as const, diagnostic: "Fix Claude config" },
]) {
  test(`Claude ${authStatus.status} state cannot be selected`, async ({
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

test("config page shows Agent defaults form", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToConfigPage(page);

  // The defaults form is the page's content; no readiness badge is shown.
  await expect(page.locator("#global-agent-provider")).toBeVisible();
  await expect(page.getByTestId("agent-readiness-badge")).toHaveCount(0);

  await waitForAnimations(page);
  const configPage = page.locator('[data-testid="onboarding-page-config"]');
  await configPage.screenshot({
    path: `${SHOTS}/04-config-defaults-form.png`,
  });
});

test("config page shows configure-later hint without Buzz Agent model config", async ({
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

  // Not-configured hint text should be visible below the form.
  await expect(
    page.getByText("You can finish now and configure agents later in Settings"),
  ).toBeVisible();

  // Take a screenshot showing the not-configured state.
  await waitForAnimations(page);
  const configPage = page.locator('[data-testid="onboarding-page-config"]');
  await configPage.screenshot({
    path: `${SHOTS}/05-setup-not-configured.png`,
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

  await page.locator("#global-agent-provider").selectOption("openai");
  await expect(
    page
      .locator("#global-agent-model")
      .getByRole("option", { name: "GPT-5.5" }),
  ).toBeAttached();
});

test("provider credentials are first-class and drive model discovery", async ({
  page,
}) => {
  await installMockBridge(
    page,
    { acpRuntimesCatalog: undefined },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToConfigPage(page);

  await page.locator("#global-agent-provider").selectOption("openai");
  const apiKey = page.getByLabel("OpenAI API Key");
  await expect(apiKey).toBeVisible();
  await apiKey.fill("test-openai-key");
  await expect(
    page
      .locator("#global-agent-model")
      .getByRole("option", { name: "GPT-5.5" }),
  ).toBeAttached();

  await page.locator("#global-agent-provider").selectOption("openai-compat");
  await expect(page.getByLabel("OpenAI API Key")).toHaveValue(
    "test-openai-key",
  );

  await page
    .locator("#global-agent-provider")
    .selectOption("__custom_provider__");
  await expect(page.getByLabel("OpenAI API Key")).not.toBeVisible();
  await expect(page.locator('input[value="test-openai-key"]')).toHaveCount(0);

  await page.locator("#global-agent-provider").selectOption("anthropic");
  await expect(page.getByLabel("Anthropic API Key")).toBeVisible();

  const databricksOption = page
    .locator("#global-agent-provider")
    .locator('option[value^="databricks"]')
    .first();
  await page
    .locator("#global-agent-provider")
    .selectOption(await databricksOption.getAttribute("value"));
  await expect(page.getByLabel("Value for DATABRICKS_HOST")).toBeVisible();
  await expect(page.getByLabel("OpenAI API Key")).not.toBeVisible();
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

  const providerSelect = page.locator("#global-agent-provider");
  await expect(providerSelect).toBeVisible();

  // First edit: select OpenAI — save starts, held open for 300 ms.
  await providerSelect.selectOption("openai");

  // Second edit before first response: select Anthropic. The coalescer must
  // persist this as the trailing save, and it must survive in the UI.
  await providerSelect.selectOption("anthropic");

  // Wait long enough for both saves to complete (2 × 300 ms + margin).
  await page.waitForTimeout(800);

  // The final provider shown must be Anthropic — neither save must overwrite
  // the later optimistic state with a stale response.
  await expect(providerSelect).toHaveValue("anthropic");
});
