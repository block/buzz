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
  await expect(next).toHaveText("Next");
  await expect(card).toHaveAttribute("aria-checked", "true");
  await expect(next).toBeEnabled();
  await next.click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("buzz-agent");
});

test("rapid harness toggles serialize the persisted preferred runtime", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("goose", { status: "not_applicable" }),
        availableRuntime("buzz-agent", { status: "not_applicable" }),
      ],
      setGlobalAgentConfigDelayMs: 200,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await page.getByTestId("onboarding-runtime-goose").click();
  await page.getByTestId("onboarding-runtime-buzz-agent").click();
  const next = page.getByTestId("onboarding-setup-next");
  await expect(next).toBeDisabled();
  await page.waitForTimeout(300);
  await expect(next).toBeDisabled();
  await expect(next).toBeEnabled({ timeout: 700 });
  expect(await readSavedRuntime(page)).toBe("buzz-agent");
});

test("authenticated Claude saves the selected runtime and routes to defaults", async ({
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
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude",
  );
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
              id: "claude-subscription",
              name: "Claude Subscription",
              description: null,
              type: "terminal",
            },
            {
              id: "anthropic-console",
              name: "Anthropic Console",
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
  await expect(
    card.getByRole("button", { name: "Claude Subscription" }),
  ).toHaveCount(0);
  await expect(
    card.getByRole("button", { name: "Anthropic Console" }),
  ).toHaveCount(0);
  await expect(
    card.getByRole("button", { name: "Log in to Claude" }),
  ).toHaveCount(0);
  await expect(
    card.getByTestId("onboarding-runtime-instructions-claude"),
  ).toBeVisible();
  await card.getByTestId("onboarding-runtime-instructions-claude").click();

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

  await expect(card.getByRole("button", { name: "Log in" })).toHaveCount(0);
  await expect(
    card.getByTestId("onboarding-runtime-instructions-codex"),
  ).toBeVisible();
  await card.getByTestId("onboarding-runtime-instructions-codex").click();
  await expect(card).toHaveAttribute("aria-checked", "true");
  await expect(card.getByText("Preferred")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
  expect(await readSavedRuntime(page)).toBe("codex");
});

test("setup-needed runtimes remain selectable", async ({ page }) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_out" }),
        availableRuntime("codex", { status: "logged_out" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await expect(page.getByTestId("onboarding-runtime-claude")).toHaveAttribute(
    "aria-disabled",
    "false",
  );
  await expect(page.getByTestId("onboarding-runtime-codex")).toHaveAttribute(
    "aria-disabled",
    "false",
  );
  await page.getByTestId("onboarding-runtime-claude").click();
  await expect(page.getByTestId("onboarding-runtime-claude")).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await page.getByTestId("onboarding-runtime-codex").click();
  await expect(page.getByTestId("onboarding-runtime-codex")).toHaveAttribute(
    "aria-checked",
    "true",
  );
});

test("Next flashes selected setup-needed runtimes instead of advancing", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_out" }),
        availableRuntime("buzz-agent", { status: "not_applicable" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const next = page.getByTestId("onboarding-setup-next");
  const claudeCard = page.getByTestId("onboarding-runtime-claude");
  const buzzCard = page.getByTestId("onboarding-runtime-buzz-agent");
  const claudeSetup = claudeCard.getByTestId(
    "onboarding-runtime-instructions-claude",
  );

  await claudeCard.click();
  await buzzCard.click();
  await expect(claudeCard).toHaveAttribute("aria-checked", "true");
  await expect(buzzCard).toHaveAttribute("aria-checked", "true");
  await expect(next).toHaveAttribute("data-soft-disabled", "true");
  await expect(next).toBeEnabled();
  await expect(page.getByTestId("onboarding-setup-next-hint")).toHaveCount(0);
  await expect(next).toHaveCSS("cursor", "default");

  await next.click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  await expect(page.getByTestId("onboarding-page-config")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next-hint")).toHaveText(
    "Please finish set up",
  );
  await expect(claudeSetup).toHaveAttribute("data-setup-flash", "true");
  await expect(
    page.getByTestId("onboarding-runtime-installed-buzz-agent"),
  ).not.toHaveAttribute("data-setup-flash", "true");
});

test("logged-out CLI runtimes can be selected and keep setup visible", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_out" }),
        availableRuntime("codex", { status: "logged_out" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  for (const runtimeId of ["claude", "codex"]) {
    const card = page.getByTestId(`onboarding-runtime-${runtimeId}`);
    await expect(
      card.getByTestId(`onboarding-runtime-instructions-${runtimeId}`),
    ).toBeVisible();
    await expect(
      page.getByTestId(`onboarding-runtime-installed-${runtimeId}`),
    ).toHaveCount(0);

    await card.click();
    await expect(card).toHaveAttribute("aria-checked", "true");
    await expect(
      card.getByTestId(`onboarding-runtime-instructions-${runtimeId}`),
    ).toBeVisible();
    await expect(
      page.getByTestId(`onboarding-runtime-installed-${runtimeId}`),
    ).toHaveCount(0);
  }
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
  await expect(
    page.getByTestId("onboarding-runtime-buzz-agent").getByRole("heading", {
      name: "Buzz",
    }),
  ).toBeVisible();
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

test("multiple CLI harnesses route to default harness selection", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_in" }),
        availableRuntime("codex", { status: "logged_in" }),
      ],
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  await page.getByTestId("onboarding-runtime-claude").click();
  await page.getByTestId("onboarding-runtime-codex").click();
  await page.getByTestId("onboarding-setup-next").click();

  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Claude",
  );
  await chooseConfigDropdownOption(
    page,
    "global-agent-default-harness",
    "codex",
  );
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Codex",
  );
  await expect.poll(() => readSavedRuntime(page)).toBe("codex");
  await expect(page.getByTestId("global-agent-provider")).toHaveCount(0);
  const modelSelect = page.getByTestId("global-agent-model");
  const effortSelect = page.getByTestId("global-agent-thinking-effort-select");
  await expect(modelSelect).toBeVisible();
  await expect(effortSelect).toBeVisible();
  await expect(modelSelect).toHaveText("Select a model");
  await modelSelect.click();
  await expect(
    page.getByTestId("global-agent-model-option-codex-mini"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
});

test("selecting all harnesses routes defaults through Buzz Agent", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_in" }),
        availableRuntime("codex", { status: "logged_in" }),
        availableRuntime("goose", { status: "not_applicable" }),
        availableRuntime("buzz-agent", { status: "not_applicable" }),
      ],
      globalAgentConfig: {
        env_vars: {
          OPENAI_COMPAT_API_KEY: "sk-test",
        },
        provider: null,
        model: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  for (const runtimeId of ["claude", "codex", "goose", "buzz-agent"]) {
    await page.getByTestId(`onboarding-runtime-${runtimeId}`).click();
    await expect(
      page.getByTestId(`onboarding-runtime-${runtimeId}`),
    ).toHaveAttribute("aria-checked", "true");
  }

  await expect(page.getByTestId("onboarding-setup-next")).toBeEnabled();
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();
  expect(await readSavedRuntime(page)).toBe("buzz-agent");

  const harnessSelect = page.getByTestId("global-agent-default-harness");
  await expect(harnessSelect).toHaveText("Buzz");
  await harnessSelect.click();
  await expect(
    page.getByTestId("global-agent-default-harness-option-claude"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-default-harness-option-codex"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-default-harness-option-goose"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-default-harness-option-buzz-agent"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-default-harness-option-codex"),
  ).toHaveCSS("opacity", "1");
  await page.getByTestId("global-agent-default-harness-option-codex").click();
  await expect(harnessSelect).toHaveAttribute("data-value", "codex");
  await expect.poll(() => readSavedRuntime(page)).toBe("codex");
  await expect(page.getByTestId("global-agent-provider")).toHaveCount(0);
  await expect(page.getByTestId("global-agent-model")).toBeVisible();
  await expect(
    page.getByTestId("global-agent-thinking-effort-select"),
  ).toBeVisible();

  await chooseConfigDropdownOption(
    page,
    "global-agent-default-harness",
    "goose",
  );
  await expect(harnessSelect).toHaveAttribute("data-value", "goose");
  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");
  await expect(page.getByTestId("global-agent-model")).toHaveAttribute(
    "data-value",
    "gpt-5.5",
  );
  await page.getByTestId("global-agent-model").click();
  await expect(page.getByTestId("global-agent-model-option-gpt-5.5")).toHaveCSS(
    "color",
    "rgb(0, 0, 0)",
  );
  await expect(page.getByTestId("global-agent-model-option-gpt-5.5")).toHaveCSS(
    "opacity",
    "1",
  );
  await page.keyboard.press("Escape");

  await chooseConfigDropdownOption(
    page,
    "global-agent-default-harness",
    "buzz-agent",
  );
  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");
  await expect(page.getByTestId("global-agent-model")).toHaveAttribute(
    "data-value",
    "gpt-5.5",
  );
});

for (const authStatus of [
  { status: "logged_out" as const },
  { status: "unknown" as const },
  { status: "config_invalid" as const, diagnostic: "Fix Claude config" },
]) {
  test(`Claude ${authStatus.status} state can be selected before setup`, async ({
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
    await expect(card).toHaveAttribute("aria-disabled", "false");
    const setupButton = card.getByTestId(
      "onboarding-runtime-instructions-claude",
    );
    if (authStatus.status === "logged_out") {
      await expect(setupButton).toBeVisible();
      await expect(
        card.getByRole("button", { name: "Log in to Claude" }),
      ).toHaveCount(0);
      await setupButton.click();
      await expect(card).toHaveAttribute("aria-checked", "true");
    } else if (authStatus.status === "unknown") {
      const error = card.getByRole("status", {
        name: /Status unavailable/,
      });
      await expect(error).toBeVisible();
      await expect(error).toHaveCSS("font-size", "12px");
      await expect(error).toHaveCSS("position", "absolute");
      await error.hover();
      await expect(page.getByRole("tooltip")).toHaveText(
        "Couldn’t verify authentication.",
      );
      await expect(
        card.getByRole("button", { name: "Check Claude again" }),
      ).toHaveText("CHECK AGAIN");
      await card.click();
      await expect(card).toHaveAttribute("aria-checked", "true");
    } else {
      const error = card.getByRole("status", {
        name: /Configuration invalid/,
      });
      await expect(error).toBeVisible();
      await expect(error).toHaveCSS("font-size", "12px");
      await expect(error).toHaveCSS("position", "absolute");
      await error.hover();
      await expect(page.getByRole("tooltip")).toHaveText(
        "Check this runtime’s configuration and try again.",
      );
      await expect(page.getByRole("tooltip")).not.toContainText(
        "Fix Claude config",
      );
      await card.click();
      await expect(card).toHaveAttribute("aria-checked", "true");
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

test("unavailable sign-in options use the compact error and tooltip pattern", async ({
  page,
}) => {
  const discoveryError = "Auth discovery failed with sensitive details";
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("claude", { status: "logged_out" }),
      ],
      acpAuthMethodsError: discoveryError,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const card = page.getByTestId("onboarding-runtime-claude");
  const setupButton = card.getByTestId(
    "onboarding-runtime-instructions-claude",
  );
  const error = card.getByRole("status", { name: /Sign-in unavailable/ });
  await expect(error).toBeVisible();
  await expect(error).toHaveCSS("font-size", "12px");
  await expect(error).toHaveCSS("position", "absolute");
  await expect(error).toHaveCSS("white-space", "nowrap");
  await expect(error).not.toHaveAttribute("title");
  await error.hover();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Couldn’t load sign-in options.",
  );
  await expect(page.getByRole("tooltip")).not.toContainText(discoveryError);
  await expect(setupButton).toBeVisible();
  await expect(setupButton).toHaveText("SET UP");
});

test("failed sign-in uses the compact error and tooltip pattern", async ({
  page,
}) => {
  const connectionError = "Terminal launch failed with sensitive details";
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
              id: "login",
              name: "Sign in",
              description: null,
              type: "terminal",
            },
          ],
        },
      },
      connectAcpRuntimeError: connectionError,
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const card = page.getByTestId("onboarding-runtime-claude");
  const setupButton = card.getByTestId(
    "onboarding-runtime-instructions-claude",
  );
  const heading = card.getByRole("heading", { name: "Claude" });
  const headingTopBefore = await heading.evaluate(
    (element) => element.getBoundingClientRect().top,
  );
  const setupTopBefore = await setupButton.evaluate(
    (element) => element.getBoundingClientRect().top,
  );

  await setupButton.click();

  const error = card.getByRole("status", { name: /Sign-in failed/ });
  await expect(error).toBeVisible();
  await expect(error).toHaveCSS("font-size", "12px");
  await expect(error).toHaveCSS("position", "absolute");
  await expect(error).toHaveCSS("white-space", "nowrap");
  await expect(error).not.toHaveAttribute("title");
  await error.hover();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Couldn’t start sign-in. Try again.",
  );
  await expect(page.getByRole("tooltip")).not.toContainText(connectionError);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("tooltip")).toBeHidden();
  await error.focus();
  const tooltip = page.getByRole("tooltip");
  await expect(tooltip).toHaveText("Couldn’t start sign-in. Try again.");
  await expect(error).toHaveAccessibleName(
    "Sign-in failed. Couldn’t start sign-in. Try again.",
  );
  const [cardBox, setupBox, tooltipBox] = await Promise.all([
    card.boundingBox(),
    setupButton.boundingBox(),
    tooltip.boundingBox(),
  ]);
  expect(cardBox).not.toBeNull();
  expect(setupBox).not.toBeNull();
  expect(tooltipBox).not.toBeNull();
  expect(tooltipBox?.y).toBeGreaterThanOrEqual(
    (cardBox?.y ?? 0) + (cardBox?.height ?? 0),
  );
  expect(tooltipBox?.y).toBeGreaterThanOrEqual(
    (setupBox?.y ?? 0) + (setupBox?.height ?? 0),
  );
  await expect(setupButton).toBeVisible();
  expect(
    await heading.evaluate((element) => element.getBoundingClientRect().top),
  ).toBe(headingTopBefore);
  expect(
    await setupButton.evaluate(
      (element) => element.getBoundingClientRect().top,
    ),
  ).toBe(setupTopBefore);
});

test("failed install pins a single-line 12px error without moving card content", async ({
  page,
}) => {
  const installError = "Install already in progress with additional details";
  await installMockBridge(
    page,
    {
      installAcpRuntimeResult: {
        success: false,
        steps: [
          {
            step: "Install adapter",
            command: "npm install adapter",
            success: false,
            stdout: "",
            stderr: installError,
            exit_code: 1,
          },
        ],
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);

  const card = page.getByTestId("onboarding-runtime-claude");
  const setupButton = page.getByTestId("onboarding-runtime-install-claude");
  const heading = card.getByRole("heading", { name: "Claude" });
  const headingTopBefore = await heading.evaluate(
    (element) => element.getBoundingClientRect().top,
  );
  const detail = card.getByText("CLI detected; ACP adapter missing.");
  const detailTopBefore = await detail.evaluate(
    (element) => element.getBoundingClientRect().top,
  );
  const setupTopBefore = await setupButton.evaluate(
    (element) => element.getBoundingClientRect().top,
  );

  await setupButton.click();

  const error = page.getByTestId("onboarding-runtime-error-claude");
  await expect(error).toBeVisible();
  await expect(error).toHaveText(/Setup failed/);
  await expect(error).not.toHaveAttribute("title");
  await expect(setupButton).toBeVisible();
  await expect(setupButton).toHaveText("SET UP");
  await expect(error).toHaveCSS("font-size", "12px");
  await expect(error).toHaveCSS("position", "absolute");
  await expect(error).toHaveCSS("white-space", "nowrap");
  await expect(error.locator("span")).toHaveCSS("text-overflow", "ellipsis");
  await error.hover();
  await expect(page.getByRole("tooltip")).toHaveText(
    "Setup couldn’t be completed. Try again.",
  );
  await expect(page.getByRole("tooltip")).not.toContainText(installError);
  expect(
    await heading.evaluate((element) => element.getBoundingClientRect().top),
  ).toBe(headingTopBefore);
  expect(
    await detail.evaluate((element) => element.getBoundingClientRect().top),
  ).toBe(detailTopBefore);
  expect(
    await setupButton.evaluate(
      (element) => element.getBoundingClientRect().top,
    ),
  ).toBe(setupTopBefore);
});

test("successful install still waits for refreshed runtime readiness", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToSetupPage(page);

  const claudeCard = page.getByTestId("onboarding-runtime-claude");
  const setupButton = page.getByTestId("onboarding-runtime-install-claude");
  await expect(setupButton).toHaveText("SET UP");
  await expect(claudeCard).toHaveAttribute("aria-checked", "false");

  await setupButton.focus();
  await page.keyboard.press("Enter");
  await expect(
    page.getByTestId("onboarding-runtime-installed-claude"),
  ).toHaveText("INSTALLED");
  const next = page.getByTestId("onboarding-setup-next");
  await expect(next).toBeEnabled();
  await expect(next).toHaveAttribute("data-soft-disabled", "true");
  await next.click();
  await expect(page.getByTestId("onboarding-page-2")).toBeVisible();
  await expect(page.getByTestId("onboarding-page-config")).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-next-hint")).toHaveText(
    "Please finish set up",
  );
});

test("config page shows Agent defaults form", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await navigateToConfigPage(page);

  // The defaults form is the page's content; no readiness badge is shown.
  await expect(page.locator("#global-agent-default-harness")).toBeVisible();
  await expect(page.getByTestId("global-agent-default-harness")).toHaveText(
    "Buzz",
  );
  await expect(page.getByText("Default harness")).toBeVisible();
  await expect(page.getByText("Provider", { exact: true })).toBeVisible();
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
    page.getByTestId("global-agent-provider-option-anthropic"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-provider-option-openai"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-provider-option-__custom_provider__"),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByText("Applies to all agents")).toHaveCount(0);
  await expect(page.getByLabel("OpenAI API Key")).toHaveCount(0);
  await expect(effortSelect).toBeVisible();
  await expect(
    page.getByText(
      "This will be set as your default model configuration across Buzz. You can always change this in your Settings or give specific agents a different configuration.",
      { exact: true },
    ),
  ).toBeVisible();
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
        env_vars: {
          BUZZ_AGENT_THINKING_EFFORT: "high",
          OPENAI_COMPAT_API_KEY: "sk-test",
        },
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

test("config page model dropdown filters options via search", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      globalAgentConfig: {
        env_vars: { OPENAI_COMPAT_API_KEY: "sk-test" },
        provider: "openai",
        model: null,
      },
    },
    {
      skipCommunitySeed: true,
      skipOnboardingSeed: true,
    },
  );
  await page.goto("/");

  await navigateToConfigPage(page);

  const modelSelect = page.getByTestId("global-agent-model");
  await expect(modelSelect).toBeEnabled();
  await modelSelect.click();
  const search = page.getByTestId("global-agent-model-search");
  await expect(search).toBeVisible();
  await expect(
    page.getByTestId("global-agent-model-option-gpt-5.5"),
  ).toBeVisible();
  await search.fill("mini");
  await expect(
    page.getByTestId("global-agent-model-option-gpt-5.4-mini"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-model-option-gpt-5.5"),
  ).toHaveCount(0);
  await search.fill("zzz-no-such-model");
  await expect(page.getByText("No matches")).toBeVisible();
  await search.fill("");
  await page.getByTestId("global-agent-model-option-gpt-5.4-nano").click();
  await expect(modelSelect).toHaveAttribute("data-value", "gpt-5.4-nano");
});

test("config page defaults model after provider selection", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      globalAgentConfig: {
        env_vars: { OPENAI_COMPAT_API_KEY: "sk-test" },
        provider: null,
        model: null,
      },
    },
    {
      skipCommunitySeed: true,
      skipOnboardingSeed: true,
    },
  );
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
        { key: "ANTHROPIC_API_KEY", masked: true, value: "••••••" },
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

  await navigateToSetupPage(page);

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

  await page.getByTestId("welcome-setup-back").click();
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
      globalAgentConfig: {
        env_vars: {
          OPENAI_COMPAT_API_KEY: "sk-test",
        },
        provider: null,
        model: null,
      },
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

test("Goose provider dropdown offers setup providers before credentials exist", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      acpRuntimesCatalog: [
        availableRuntime("goose", { status: "not_applicable" }),
      ],
      globalAgentConfig: {
        env_vars: {},
        provider: null,
        model: null,
      },
    },
    { skipCommunitySeed: true, skipOnboardingSeed: true },
  );
  await page.goto("/");
  await navigateToSetupPage(page);
  await page.getByTestId("onboarding-runtime-goose").click();
  await page.getByTestId("onboarding-setup-next").click();
  await expect(page.getByTestId("onboarding-page-config")).toBeVisible();

  await page.getByTestId("global-agent-provider").click();
  await expect(
    page.getByTestId("global-agent-provider-option-anthropic"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-provider-option-openai"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-provider-option-relay-mesh"),
  ).toHaveCount(0);
});

test("compact default config still persists rapid provider edits", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      globalAgentConfig: {
        env_vars: {
          ANTHROPIC_API_KEY: "sk-test",
          OPENAI_COMPAT_API_KEY: "sk-test",
        },
        provider: null,
        model: null,
      },
    },
    {
      skipCommunitySeed: true,
      skipOnboardingSeed: true,
    },
  );
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

  await expect(page.getByLabel("Anthropic API Key")).toBeVisible();
  await expect(page.getByLabel("OpenAI API Key")).toHaveCount(0);
  await expect(page.getByLabel("Value for DATABRICKS_HOST")).toHaveCount(0);
});

test("compact default config keeps credential-backed providers after selecting Buzz", async ({
  page,
}) => {
  await installMockBridge(
    page,
    {
      globalAgentConfig: {
        env_vars: {
          ANTHROPIC_API_KEY: "sk-test",
          OPENAI_COMPAT_API_KEY: "sk-test",
        },
        provider: null,
        model: null,
      },
    },
    {
      skipCommunitySeed: true,
      skipOnboardingSeed: true,
    },
  );
  await page.goto("/");
  await navigateToConfigPage(page);

  const providerSelect = page.getByTestId("global-agent-provider");
  await chooseConfigDropdownOption(page, "global-agent-provider", "openai");
  await expect(providerSelect).toHaveAttribute("data-value", "openai");

  await chooseConfigDropdownOption(page, "global-agent-provider", "relay-mesh");
  await expect(providerSelect).toHaveAttribute("data-value", "relay-mesh");

  await providerSelect.click();
  await expect(
    page.getByTestId("global-agent-provider-option-anthropic"),
  ).toBeVisible();
  await expect(
    page.getByTestId("global-agent-provider-option-openai"),
  ).toBeVisible();
  await page.getByTestId("global-agent-provider-option-anthropic").click();
  await expect(providerSelect).toHaveAttribute("data-value", "anthropic");
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
      globalAgentConfig: {
        env_vars: {
          ANTHROPIC_API_KEY: "sk-test",
          OPENAI_COMPAT_API_KEY: "sk-test",
        },
        provider: null,
        model: null,
      },
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
