import { expect, test } from "@playwright/test";

import { installMockBridge, openCreateChannelDialog } from "../helpers/bridge";

const DEFAULT_AGENT_ACTIVITY_PUBKEY =
  "db0b028cd36f4d3e36c8300cce87252c1f7fc9495ffecc53f393fcac341ffd36";

async function getTimelineMetrics(page: import("@playwright/test").Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const timeline = element as HTMLDivElement;

    return {
      clientHeight: timeline.clientHeight,
      scrollHeight: timeline.scrollHeight,
      scrollTop: timeline.scrollTop,
      distanceFromBottom:
        timeline.scrollHeight - timeline.clientHeight - timeline.scrollTop,
    };
  });
}

async function ensureTimelineScrollable(
  page: import("@playwright/test").Page,
  prefix: string,
) {
  const input = page.getByTestId("message-input");
  const sendButton = page.getByTestId("send-message");

  for (let index = 0; index < 24; index += 1) {
    const metrics = await getTimelineMetrics(page);
    if (metrics.scrollHeight > metrics.clientHeight + 160) {
      return;
    }

    const message = `${prefix} seed ${index}`;

    await input.fill(message);
    await sendButton.click();
    await expect(page.getByTestId("message-timeline")).toContainText(message);
  }

  const metrics = await getTimelineMetrics(page);
  expect(metrics.scrollHeight).toBeGreaterThan(metrics.clientHeight + 160);
}

async function focusSidebarSearchWithShortcut(
  page: import("@playwright/test").Page,
) {
  const openSearchButton = page.getByTestId("open-search");

  await expect(openSearchButton).toBeVisible();
  await page.evaluate(() => {
    const isMac = /mac|iphone|ipad|ipod/i.test(navigator.platform);
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        code: "KeyK",
        ctrlKey: !isMac,
        key: "k",
        metaKey: isMac,
      }),
    );
  });
  await expect(page.getByTestId("search-results")).toBeVisible();
  await expect(page.getByTestId("search-dialog-input")).toBeFocused();
}

async function expectHomeView(page: import("@playwright/test").Page) {
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
}

async function selectHomeInboxFilter(
  page: import("@playwright/test").Page,
  label: "Activity" | "Agents",
) {
  await page
    .getByTestId("home-inbox")
    .getByRole("button", {
      name: /^Filter inbox:/,
    })
    .click();
  await page.getByRole("menuitemradio", { name: label }).click();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("loads the app shell with mocked channels", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(page.getByTestId("stream-list")).toContainText("general");
  await expect(page.getByTestId("forum-list")).toContainText("watercooler");
  await expect(page.getByTestId("dm-list")).toContainText("alice-tyler");
});

async function chooseSharedComputeProvider(
  page: import("@playwright/test").Page,
) {
  await page.getByRole("tab", { name: "Customize for this agent" }).click();
  const provider = page.locator("#persona-llm-provider");
  await expect(provider).toBeVisible({ timeout: 10_000 });
  await provider.press("Enter");
  await page
    .getByRole("menuitemradio", {
      exact: true,
      name: "Buzz shared compute",
    })
    .click();
}

test("creates a new mocked stream", async ({ page }) => {
  const channelName = `release-notes-${Date.now()}`;

  await page.goto("/");
  await openCreateChannelDialog(page);
  await page.getByTestId("create-channel-name").fill(channelName);
  await page
    .getByTestId("create-channel-description")
    .fill("Release coordination");
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toContainText(channelName);
});

test("Buzz shared compute explains automatic model selection", async ({
  page,
}) => {
  await page.goto("/");
  await page.evaluate(() => {
    (
      window as Window & {
        __BUZZ_E2E_SET_MESH__?: (mesh: {
          models?: Array<{ id: string; name: string | null }>;
        }) => void;
      }
    ).__BUZZ_E2E_SET_MESH__?.({ models: [] });
  });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();
  await chooseSharedComputeProvider(page);

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
            .__BUZZ_E2E_COMMANDS__ ?? [],
      ),
    )
    .toContain("discover_agent_models");
  await expect(page.locator("#persona-model")).toContainText("Automatic");
  await expect(
    page.getByText(
      "Buzz will choose an available shared model when the agent starts.",
    ),
  ).toBeVisible();
  await expect(page.locator("#persona-custom-model")).toHaveCount(0);
});

test("create agent persists Buzz shared compute with auto model", async ({
  page,
}) => {
  const agentName = `Shared compute agent ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();
  await page.locator("#persona-display-name").fill(agentName);

  await chooseSharedComputeProvider(page);

  const model = page.locator("#persona-model");
  await expect(model).toContainText("Automatic");
  await page.getByTestId("persona-dialog-submit").click();
  await expect(
    page.getByRole("heading", { name: "Agent created" }),
  ).toBeVisible({ timeout: 10_000 });

  const createPayload = await page.evaluate((name) => {
    const log = (
      window as Window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: unknown;
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return log
      ?.filter((entry) => entry.command === "create_managed_agent")
      .map((entry) => entry.payload as { input?: Record<string, unknown> })
      .find((payload) => payload.input?.name === name)?.input;
  }, agentName);

  expect(createPayload).toMatchObject({
    agentCommand: "buzz-agent",
    model: "auto",
    provider: "relay-mesh",
    spawnAfterCreate: true,
    startOnAppLaunch: true,
  });
});

test("create agent supports parallelism and system prompt overrides", async ({
  page,
}) => {
  const agentName = `Parallel agent ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  await page.getByRole("menuitem", { name: /^New agent$/ }).click();

  await page.locator("#persona-display-name").fill(agentName);
  await page
    .locator("#persona-system-prompt")
    .fill("You are concise and parallelize independent work.");

  // The buzz-agent runtime auto-selects once the ACP runtime catalog loads;
  // Customize reveals the per-agent LLM provider and model fields.
  await page.getByRole("tab", { name: "Customize for this agent" }).click();
  const llmProvider = page.locator("#persona-llm-provider");
  await expect(llmProvider).toBeVisible({ timeout: 10_000 });
  await llmProvider.press("Enter");
  await page
    .getByRole("menuitemradio", { exact: true, name: "Anthropic" })
    .click();
  const model = page.locator("#persona-model");
  await model.click();
  await page
    .getByRole("button", { name: "Custom model...", exact: true })
    .click();
  await page.getByLabel("Custom model ID").fill("claude-opus-4-5");
  await page.getByLabel("Anthropic API Key").fill("sk-test-api-key-for-e2e");

  const advancedToggle = page.getByRole("button", {
    name: "Advanced",
    exact: true,
  });
  await advancedToggle.click();
  // Parallelism is above the env-vars editor in the Advanced section; filling
  // the required API-key row may have scrolled the dialog past it. Scroll back.
  await page
    .locator("#persona-parallelism")
    .evaluate((el) => el.scrollIntoView({ block: "nearest" }));
  await expect(page.locator("#persona-parallelism")).toBeVisible();
  await page.locator("#persona-parallelism").fill("3");

  // Submitting mints a running instance whose behavioral quad resolves from
  // the definition (agents always start after creation).
  await page.getByTestId("persona-dialog-submit").click();

  await expect(
    page.getByRole("heading", { name: "Agent created" }),
  ).toBeVisible({ timeout: 10_000 });
  await page.getByRole("button", { name: "Done" }).click();

  await expect(page.getByTestId("agents-library-personas")).toContainText(
    agentName,
  );

  // Logs now live in the profile sidebar (PR #1274), not an inline panel.
  // Open the new agent's card to reveal the profile panel, then read the
  // harness log from the diagnostics view.
  await page
    .getByRole("button", { name: `${agentName} agent profile` })
    .click();
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();

  await page.getByTestId("user-profile-tab-runtime").click();
  await page.getByTestId("user-profile-diagnostics-ingress").click();

  const log = page.getByTestId("managed-agent-log-content");
  await expect(log).toContainText("parallelism=3");
  await expect(log).toContainText("system prompt override configured");
});

test("opens a mocked channel from the inbox feed", async ({ page }) => {
  const inboxList = page.getByTestId("home-inbox-list");

  await page.goto("/");

  await expectHomeView(page);
  await expect(inboxList).toContainText("Please review the release checklist.");

  const releaseRow = page.getByTestId("home-inbox-item-mock-feed-mention");
  await releaseRow.hover();
  await releaseRow.getByRole("button", { name: "Open in channel" }).click();

  await expect(page).toHaveURL(
    /#\/channels\/9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50\?messageId=mock-feed-mention$/,
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("inbox feed shows channel and agent activity sections", async ({
  page,
}) => {
  const inboxList = page.getByTestId("home-inbox-list");

  await page.goto("/");

  await selectHomeInboxFilter(page, "Activity");
  await expect(inboxList).toContainText(
    "Engineering shipped the desktop build.",
  );

  await selectHomeInboxFilter(page, "Agents");
  await expect(inboxList).toContainText(
    "Agent progress: channel index complete.",
  );
  await inboxList.getByText("Agent progress: channel index complete.").click();
  await expect(page.getByTestId("home-inbox-detail")).toContainText(
    "Agent progress: channel index complete.",
  );
});

test("inbox agent hover hides actions without agent access", async ({
  page,
}) => {
  await page.goto("/");

  await selectHomeInboxFilter(page, "Agents");
  const agentRow = page.getByTestId("home-inbox-item-mock-feed-agent");
  await expect(agentRow).toContainText(
    "Agent progress: channel index complete.",
  );

  await agentRow.getByTestId("home-inbox-avatar-mock-feed-agent").hover();
  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-message-${DEFAULT_AGENT_ACTIVITY_PUBKEY}`,
    ),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-wave-${DEFAULT_AGENT_ACTIVITY_PUBKEY}`,
    ),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-huddle-${DEFAULT_AGENT_ACTIVITY_PUBKEY}`,
    ),
  ).toHaveCount(0);
});

test("opens a mocked forum activity item from the inbox feed", async ({
  page,
}) => {
  await page.goto("/");

  await selectHomeInboxFilter(page, "Activity");
  await expect(page.getByTestId("home-inbox-list")).toContainText(
    "Engineering shipped the desktop build.",
  );
  await page
    .getByTestId("home-inbox-list")
    .getByText("Engineering shipped the desktop build.")
    .click();
  await expect(page.getByTestId("home-inbox-detail")).toContainText(
    "Engineering shipped the desktop build.",
  );
});

test("inbox feed renders resolved author labels", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("home-inbox-list")).toContainText("alice");
  await expect(page.getByTestId("home-inbox-list")).not.toContainText("You");
});

test("opens sidebar search with the shortcut and loads the exact result", async ({
  page,
}) => {
  await page.goto("/");

  await focusSidebarSearchWithShortcut(page);

  await page.getByTestId("search-dialog-input").fill("shipped");
  await expect(page.getByTestId("search-results")).toContainText(
    "Engineering shipped the desktop build.",
  );

  await page.keyboard.press("Enter");

  await expect(page).toHaveURL(
    /#\/channels\/1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9\?messageId=mock-engineering-shipped$/,
  );
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Engineering shipped the desktop build.",
  );
});

test("opens channel matches from search", async ({ page }) => {
  await page.goto("/");

  await focusSidebarSearchWithShortcut(page);

  await page.getByTestId("search-dialog-input").fill("engineering");
  const results = page.getByTestId("search-results");

  await expect(results).toContainText("engineering");
  await expect(results).toContainText("Engineering discussions");
  await expect(results).toContainText(
    "Design system and UX discussions with engineering partners",
  );
  await expect(
    results.locator('[data-testid^="search-result-channel-"]').first(),
  ).toHaveAttribute(
    "data-testid",
    "search-result-channel-1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
  );

  await expect(
    results.getByTestId(
      "search-result-channel-1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9",
    ),
  ).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Enter");

  await expect(page).toHaveURL(
    /#\/channels\/1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9$/,
  );
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
});

test("closes sidebar search with Escape", async ({ page }) => {
  await page.goto("/");

  await focusSidebarSearchWithShortcut(page);
  await page.getByTestId("search-dialog-input").fill("shipped");

  await page.keyboard.press("Escape");

  await expect(page.getByTestId("search-results")).toHaveCount(0);
  await expect(page.getByTestId("open-search")).toBeFocused();
});

test("search shortcut opens search without disturbing the collapsed sidebar", async ({
  page,
}) => {
  await page.goto("/");

  await expect(page.getByTestId("open-search")).toBeVisible();

  const sidebarRoot = page.locator('[data-side="left"][data-state]');
  await expect(sidebarRoot).toHaveAttribute("data-state", "expanded");

  // Collapse the sidebar; its pinned-header search slides off-screen.
  await page
    .getByRole("button", { name: "Toggle Sidebar", exact: true })
    .click();
  await expect(sidebarRoot).toHaveAttribute("data-state", "collapsed");

  await page.evaluate(() => {
    const isMac = /mac|iphone|ipad|ipod/i.test(navigator.platform);
    window.dispatchEvent(
      new KeyboardEvent("keydown", {
        bubbles: true,
        cancelable: true,
        code: "KeyK",
        ctrlKey: !isMac,
        key: "k",
        metaKey: isMac,
      }),
    );
  });

  // Search opens in its portal dialog; the sidebar must not react.
  await expect(page.getByTestId("search-dialog-input")).toBeFocused();
  await expect(sidebarRoot).toHaveAttribute("data-state", "collapsed");
});

test("search results use your resolved profile label instead of You", async ({
  page,
}) => {
  await page.goto("/");

  await focusSidebarSearchWithShortcut(page);

  await page.getByTestId("search-dialog-input").fill("welcome");
  const results = page.getByTestId("search-results");

  await expect(results).toContainText("Welcome to #general");
  await expect(results).toContainText("npub1mock...");
  await expect(results).not.toContainText("You");
});

test("opens accessible unjoined channels from search in read-only mode", async ({
  page,
}) => {
  await page.goto("/");

  await focusSidebarSearchWithShortcut(page);

  await page.getByTestId("search-dialog-input").fill("critique");
  const results = page.getByTestId("search-results");

  await expect(results).toContainText(
    "Design critique notes for the browse flow.",
  );
  await results.getByText("Design critique notes for the browse flow.").click();

  await expect(page.getByTestId("chat-title")).toHaveText("design");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Design critique notes for the browse flow.",
  );
  await expect(page.getByTestId("join-banner")).toBeVisible();
});

test("replaces the channel pane when switching channels", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(page.getByTestId("message-channel-intro")).toContainText(
    "This is the beginning of the regular channel.",
  );
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    "Welcome to #general",
  );
  await expect(page.getByTestId("message-timeline")).toHaveCount(1);
  await expect(page.getByTestId("message-timeline-day-divider")).toHaveCount(0);

  await page.getByTestId("channel-engineering").click();
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page.getByTestId("message-channel-intro")).toBeVisible();
  await expect(page.getByTestId("message-timeline")).toHaveCount(1);
  await expect(page.getByTestId("message-timeline-day-divider")).toHaveCount(0);
});

test("sends a mocked channel message", async ({ page }) => {
  const message = `Smoke message ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
  await expect
    .poll(() =>
      page.evaluate(() => {
        const row = Array.from(
          document.querySelectorAll<HTMLElement>("[data-message-id]"),
        ).at(-1);
        const composer = document.querySelector<HTMLElement>(
          '[data-testid="message-composer"]',
        );
        if (!row || !composer) return Number.NEGATIVE_INFINITY;
        return (
          composer.getBoundingClientRect().top -
          row.getBoundingClientRect().bottom
        );
      }),
    )
    .toBeGreaterThanOrEqual(0);
});

test("supports multiline drafts with Ctrl+Enter and sends with Enter", async ({
  page,
}) => {
  const firstLine = `Shortcut smoke line one ${Date.now()}`;
  const restOfLines = [
    "Shortcut smoke line two",
    "Shortcut smoke line three",
    "Shortcut smoke line four",
    "Shortcut smoke line five",
  ];
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(
    page.getByRole("button", { name: "Send message" }),
  ).toBeVisible();
  const initialInputHeight = await input.evaluate(
    (element) => (element as HTMLElement).clientHeight,
  );
  expect(initialInputHeight).toBeLessThan(40);
  await input.fill(firstLine);
  for (const line of restOfLines) {
    await input.press("Shift+Enter");
    await input.pressSequentially(line);
  }
  for (const line of [firstLine, ...restOfLines]) {
    await expect(input).toContainText(line);
  }
  const expandedInputHeight = await input.evaluate(
    (element) => (element as HTMLElement).clientHeight,
  );
  expect(expandedInputHeight).toBeLessThanOrEqual(130);
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    firstLine,
  );
  await input.press("Enter");

  await expect(page.getByTestId("message-timeline")).toContainText(firstLine);
  await expect(page.getByTestId("message-timeline")).toContainText(
    restOfLines[restOfLines.length - 1],
  );
});

test("does not shift the timeline when the composer grows", async ({
  page,
}) => {
  const input = page.getByTestId("message-input");
  const prefix = `Composer growth ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await ensureTimelineScrollable(page, prefix);
  await page.waitForTimeout(400);
  await page.getByTestId("message-timeline").evaluate((element) => {
    const timeline = element as HTMLDivElement;
    timeline.scrollTop = 0;
    timeline.dispatchEvent(new Event("scroll"));
  });
  await expect
    .poll(async () => (await getTimelineMetrics(page)).distanceFromBottom)
    .toBeGreaterThan(160);
  const before = await getTimelineMetrics(page);

  await input.fill("Composer growth line one");
  await input.press("Shift+Enter");
  await input.pressSequentially("Composer growth line two");
  await input.press("Shift+Enter");
  await input.pressSequentially("Composer growth line three");
  await input.press("Shift+Enter");
  await input.pressSequentially("Composer growth line four");

  await page.waitForTimeout(1200);

  const after = await getTimelineMetrics(page);
  expect(after.clientHeight).toBeLessThanOrEqual(before.clientHeight);
  expect(Math.abs(after.scrollTop - before.scrollTop)).toBeLessThanOrEqual(2);
  expect(after.distanceFromBottom).toBeGreaterThan(160);
});
