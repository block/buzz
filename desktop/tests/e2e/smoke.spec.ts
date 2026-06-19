import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

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
  await expect(openSearchButton).toBeFocused();
  await expect(page.getByTestId("search-results")).toBeVisible();
  await expect(page.getByTestId("search-dialog")).toHaveCount(0);
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
      name: /^(All|Mentions|Needs Action|Activity|Agents)$/,
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

test("creates a new mocked stream", async ({ page }) => {
  const channelName = `release-notes-${Date.now()}`;

  await page.goto("/");
  await page.getByRole("button", { name: "Create a channel" }).click();
  await page.getByTestId("create-channel-name").fill(channelName);
  await page
    .getByTestId("create-channel-description")
    .fill("Release coordination");
  await page.getByTestId("create-channel-submit").click();

  await expect(page.getByTestId("stream-list")).toContainText(channelName);
  await expect(page.getByTestId("chat-title")).toContainText(channelName);
});

test("create agent supports parallelism and system prompt overrides", async ({
  page,
}) => {
  const agentName = `Parallel agent ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("open-agents-view").click();
  await page
    .getByTestId("agents-library-personas")
    .getByRole("button", { name: "New", exact: true })
    .click();
  await page.getByText("Custom Agent").click();

  await page.getByTestId("agent-name-input").fill(agentName);
  await page.getByRole("button", { name: "Advanced setup" }).click();
  await page.getByTestId("agent-parallelism-input").fill("3");
  await page
    .getByTestId("agent-system-prompt-input")
    .fill("You are concise and parallelize independent work.");
  await page.getByTestId("create-agent-submit").click();

  await expect(
    page.getByRole("heading", { name: "Agent created" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();

  await expect(page.getByTestId("agents-library-personas")).toContainText(
    agentName,
  );
  const inlineLog = page
    .getByTestId("agents-library-personas")
    .getByTestId("managed-agent-log-content");

  await expect(inlineLog).toContainText("parallelism=3");
  await expect(inlineLog).toContainText("system prompt override configured");
});

test("opens a mocked channel from the home feed", async ({ page }) => {
  const inboxList = page.getByTestId("home-inbox-list");

  await page.goto("/");

  await expectHomeView(page);
  await expect(inboxList).toContainText("Please review the release checklist.");

  await inboxList
    .getByText("Please review the release checklist.")
    .first()
    .click();
  await page.getByRole("button", { name: "Open channel" }).click();

  await expect(page).toHaveURL(
    /#\/channels\/9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50$/,
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");
});

test("home feed shows channel and agent activity sections", async ({
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

test("opens a mocked forum activity item from the home feed", async ({
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

test("home feed renders resolved author labels", async ({ page }) => {
  await page.goto("/");

  await expect(page.getByTestId("home-inbox-list")).toContainText("alice");
  await expect(page.getByTestId("home-inbox-list")).not.toContainText("You");
});

test("opens sidebar search with the shortcut and loads the exact result", async ({
  page,
}) => {
  await page.goto("/");

  await focusSidebarSearchWithShortcut(page);

  await page.getByTestId("open-search").fill("shipped");
  await expect(page.getByTestId("search-results")).toContainText(
    "Engineering shipped the desktop build.",
  );

  await page
    .getByTestId("search-results")
    .getByText("Engineering shipped the desktop build.")
    .click();

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

  await page.getByTestId("open-search").fill("engineering");
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

  await results
    .getByTestId("search-result-channel-1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9")
    .click();

  await expect(page).toHaveURL(
    /#\/channels\/1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9$/,
  );
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
});

test("reopens the collapsed sidebar when the search shortcut fires", async ({
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

  await focusSidebarSearchWithShortcut(page);

  // The shortcut reveals the sidebar and focuses the search input.
  await expect(sidebarRoot).toHaveAttribute("data-state", "expanded");
  await expect(page.getByTestId("open-search")).toBeFocused();
});

test("search results use your resolved profile label instead of You", async ({
  page,
}) => {
  await page.goto("/");

  await focusSidebarSearchWithShortcut(page);

  await page.getByTestId("open-search").fill("welcome");
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

  await page.getByTestId("open-search").fill("critique");
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
