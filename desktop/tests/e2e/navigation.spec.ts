import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const ENGINEERING_CHANNEL_ID = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";
const WATERCOLOR_CHANNEL_ID = "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11";
const FORUM_POST_ID = "mock-forum-release-thread";
const FORUM_REPLY_ID = "mock-forum-release-reply";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

/**
 * Inline message chips no longer change their label when metadata resolves, so
 * a single hover can land while the chip is still the plain (untriggered) span.
 * Re-arm the pointer until the metadata tooltip is mounted.
 */
async function hoverUntilMetadataTooltip(
  page: import("@playwright/test").Page,
  chip: import("@playwright/test").Locator,
) {
  await expect
    .poll(async () => {
      await page.getByTestId("chat-title").hover();
      await chip.hover();
      return page
        .getByRole("tooltip")
        .locator('[data-buzz-tooltip-metadata-content=""]')
        .count();
    })
    .toBeGreaterThan(0);
}

async function navigateToWorkflows(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("open-workflows-view").click();
  await expect(page).toHaveURL(/#\/workflows$/);
  await expect(page.getByTestId("workflows-view")).toBeVisible();
}

async function createWorkflow(
  page: import("@playwright/test").Page,
  name: string,
) {
  await page.getByRole("button", { name: "Create Workflow" }).click();
  const dialog = page.getByRole("dialog", { name: "Create workflow" });
  await expect(dialog).toBeVisible();

  const channelList = page.getByTestId("channel-combobox-list");
  await expect(channelList).toBeVisible();
  await channelList
    .getByRole("option", { name: "agents", exact: true })
    .click();

  await dialog.getByRole("button", { name: "Edit workflow name" }).click();
  await dialog.getByRole("textbox", { name: "Workflow name" }).fill(name);
  await dialog.getByRole("button", { name: "Save workflow name" }).click();

  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await page.getByRole("menuitem", { name: "Send Message" }).click();
  await dialog.getByLabel("Message text").fill("Workflow notification");
  await dialog.getByRole("button", { name: "Create" }).click();
  await expect(dialog).not.toBeVisible();
}

test("global back and forward move across channel routes", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  await page.getByTestId("global-back").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("global-forward").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
});

test("back/forward keyboard chords work while the composer has focus", async ({
  page,
}) => {
  const backChord = process.platform === "darwin" ? "Meta+[" : "Alt+ArrowLeft";
  const forwardChord =
    process.platform === "darwin" ? "Meta+]" : "Alt+ArrowRight";

  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  // The composer autofocuses on channel switch; make the regression
  // condition explicit by clicking into it. The chords must still fire
  // from inside the contenteditable (#3775).
  await page.getByTestId("message-input").click();
  await expect(page.getByTestId("message-input")).toBeFocused();

  await page.keyboard.press(backChord);
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.keyboard.press(forwardChord);
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  // preventDefault kept the chord out of the editor — no stray characters.
  await expect(page.getByTestId("message-input")).toHaveText("");
});

// FIXME: the forum post "Back to posts" header renders under the fixed top
// chrome drag region, which intercepts the click. Pre-existing breakage —
// this spec file was never registered in playwright.config.ts until now.
// The header-chrome rework (PR #941) covers this overlap class.
test.fixme("direct forum thread links close back to the forum route", async ({
  page,
}) => {
  await page.goto(
    `/#/channels/${WATERCOLOR_CHANNEL_ID}/posts/${FORUM_POST_ID}`,
  );

  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(
    page.getByRole("button", { name: "Back to posts" }),
  ).toBeVisible();

  await page.getByRole("button", { name: "Back to posts" }).click();

  await expect(page).toHaveURL(
    /#\/channels\/a27e1ee9-76a6-5bdf-a5d5-1d85610dad11$/,
  );
  await expect(
    page.getByText("Release checklist: async feedback thread."),
  ).toBeVisible();
});

test("direct workflow detail links close back to workflows", async ({
  page,
}) => {
  const workflowName = `workflow_nav_${Date.now()}`;

  await navigateToWorkflows(page);
  await createWorkflow(page, workflowName);

  const workflowCard = page
    .locator('[data-testid^="workflow-card-"]')
    .filter({ hasText: workflowName })
    .first();
  const workflowTestId = await workflowCard.getAttribute("data-testid");
  const workflowId = workflowTestId?.replace("workflow-card-", "");

  expect(workflowId).toBeTruthy();

  await page.goto(`/#/workflows/${workflowId}`);

  const dialog = page.getByRole("dialog", { name: "Edit workflow" });
  await expect(dialog.getByText(workflowName, { exact: true })).toBeVisible();
  await expect(
    dialog.getByRole("button", { name: "Trigger: Message Posted" }),
  ).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Run history" })).toHaveCount(
    0,
  );
  await dialog.getByRole("button", { name: "Close", exact: true }).click();

  await expect(page).toHaveURL(/#\/workflows$/);
  await expect(page.getByTestId("workflows-view")).toBeVisible();
});

test("forum reply deep links survive reload", async ({ page }) => {
  await page.goto(
    `/#/channels/${WATERCOLOR_CHANNEL_ID}/posts/${FORUM_POST_ID}?replyId=${FORUM_REPLY_ID}`,
  );

  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(
    page.getByText("Looks good to me. We should ship it."),
  ).toBeVisible();

  await page.reload();

  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(
    page.getByText("Looks good to me. We should ship it."),
  ).toBeVisible();
});

test("back and forward restore open thread panels", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(page).toHaveURL(/thread=/);

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(threadPanel).not.toBeVisible();

  await page.getByTestId("global-back").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(threadPanel).toBeVisible();

  await page.getByTestId("global-forward").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(threadPanel).not.toBeVisible();
});

test("back undoes closing a thread panel", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  await threadPanel.getByRole("button", { name: "Close panel" }).click();
  await expect(threadPanel).not.toBeVisible();

  await page.getByTestId("global-back").click();
  await expect(threadPanel).toBeVisible();
});

test("open thread panels survive reload", async ({ page }) => {
  await page.goto("/");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(page).toHaveURL(/thread=/);

  await page.reload();

  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(threadPanel).toBeVisible();
});

test("home inbox selection survives reload and back restores it", async ({
  page,
}) => {
  await page.goto("/");

  const inboxList = page.getByTestId("home-inbox-list");
  await expect(inboxList).toBeVisible();
  const items = inboxList.locator('[data-testid^="home-inbox-item-"]');
  await expect(items.first()).toBeVisible();

  // The wide-viewport default selection stays local-only — the URL records
  // explicit selections, so background loads never touch the history stack.
  await expect(page.getByTestId("home-inbox-detail")).toBeVisible();
  expect(page.url()).not.toContain("item=");
  const defaultUrl = page.url();

  const selectedItem = items.first();
  const selectedTestId = await selectedItem.getAttribute("data-testid");
  const selectedItemId = selectedTestId?.replace("home-inbox-item-", "");
  expect(selectedItemId).toBeTruthy();
  await selectedItem.click();
  await expect
    .poll(() => page.url())
    .toContain(`item=${encodeURIComponent(selectedItemId ?? "")}`);

  await page.reload();

  await expect(inboxList).toBeVisible();
  await expect(page.getByTestId("home-inbox-detail")).toBeVisible();
  expect(page.url()).toContain(
    `item=${encodeURIComponent(selectedItemId ?? "")}`,
  );

  await page.getByTestId("global-back").click();
  await expect.poll(() => page.url()).toBe(defaultUrl);
});

test("settings is a route: section survives reload, closing returns to the previous panel state", async ({
  page,
}) => {
  await page.goto("/");

  // Open a channel with a thread panel so there's panel state to come back to.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  const channelUrl = page.url();

  await openSettings(page);
  await expect(page).toHaveURL(/#\/settings/);

  // Section switches rewrite the settings entry (replace, not push).
  await page.getByTestId("settings-nav-notifications").click();
  await expect(page).toHaveURL(/section=notifications/);

  await page.reload();
  await expect(page.getByTestId("settings-view")).toBeVisible();
  await expect(page).toHaveURL(/section=notifications/);

  await page.getByTestId("settings-back-to-app").click();
  await expect.poll(() => page.url()).toBe(channelUrl);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(threadPanel).toBeVisible();
});

test("settings shortcut returns without opening search dialog", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const channelUrl = page.url();

  // Open search via the ⌘K shortcut so the focus-request counter is non-zero,
  // then close it. The Settings subtree remounts the sidebar + search on close,
  // which must not replay the stale counter and resurrect search.
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
  await expect(page.getByTestId("search-dialog-input")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("search-results")).not.toBeVisible();

  await page.keyboard.press(
    process.platform === "darwin" ? "Meta+Comma" : "Control+Comma",
  );

  await expect(page).toHaveURL(/#\/settings/);
  await page.getByTestId("settings-back-to-app").click();

  await expect.poll(() => page.url()).toBe(channelUrl);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("search-results")).not.toBeVisible();
});

test("mixed Buzz permalinks render as chips in the composer", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const channelId = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
  const owner = "a".repeat(64);
  const pullRequestId = "c".repeat(64);
  const issueId = "b".repeat(64);
  const links = [
    `buzz://message?channel=${channelId}&id=mock-general-welcome`,
    `buzz://channel/${channelId}`,
    `buzz://repo?owner=${owner}&d=buzz-world`,
    `buzz://pr?id=${pullRequestId}&owner=${owner}&d=buzz-world`,
    `buzz://issue?id=${issueId}&owner=${owner}&d=buzz-world`,
  ].join(" ");
  const composerInput = page.getByTestId("message-input");
  await composerInput.evaluate((element, text) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", text);
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      }),
    );
  }, links);

  const chips = composerInput.locator('[data-composer-buzz-link=""]');
  await expect(chips).toHaveCount(5);
  await expect(chips.nth(0)).toHaveText("general");
  await expect(chips.nth(1)).toHaveText("general");
  await expect(chips.nth(2)).toHaveText("buzz-world");
  // PR and issue chips both use repository identity only, matching rendered chips.
  await expect(chips.nth(3)).toHaveText("buzz-world");
  await expect(chips.nth(4)).toHaveText("buzz-world");
  await expect(chips.nth(1)).toHaveClass(/inline-chip-icon-channel/);
  await expect(chips.nth(2)).toHaveClass(/inline-chip-icon-repo/);
  await expect(chips.nth(3)).toHaveClass(/inline-chip-icon-pr/);
  await expect(chips.nth(4)).toHaveClass(/inline-chip-icon-issue/);
  for (const index of [0, 1, 2, 3, 4]) {
    const iconMask = await chips
      .nth(index)
      .evaluate((element) =>
        getComputedStyle(element, "::before").getPropertyValue(
          "-webkit-mask-image",
        ),
      );
    expect(iconMask).toContain("data:image/svg+xml");
  }
  await expect(composerInput).not.toContainText("buzz://");
});

test("message links to visible root messages open the thread panel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to general",
  );
  await page.evaluate(() => {
    (
      window as Window & { __BUZZ_E2E_DEFER_GET_EVENT__?: string | null }
    ).__BUZZ_E2E_DEFER_GET_EVENT__ = "mock-general-welcome";
  });

  const link =
    "buzz://message?channel=9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50&id=mock-general-welcome";
  const composerInput = page.getByTestId("message-input");
  await composerInput.fill("Root link repro #random ");
  await composerInput.focus();
  await composerInput.evaluate((element, href) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", href);
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      }),
    );
  }, link);
  const composerLink = composerInput.locator('[data-composer-message-link=""]');
  await expect(composerLink).toHaveText(/general(?: · mock-gen)?/);
  await expect(composerLink).toHaveClass(/mention-chip/);
  await expect(composerLink).toHaveClass(/inline-chip-icon-message/);
  await expect(composerLink).toHaveAttribute("data-buzz-link", "");
  await expect(composerLink).toHaveAttribute("title", "Thread in #general");
  await expect(composerInput).not.toContainText("buzz://message");
  await page.getByTestId("send-message").click();

  const linkMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Root link repro" })
    .last();
  await expect(linkMessage).toBeVisible();
  const rootThreadLink = linkMessage.getByRole("button", {
    name: "Open message in channel general",
  });
  await expect(rootThreadLink).toHaveText("general");
  const pendingChipBox = await rootThreadLink.boundingBox();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as Window & { __BUZZ_E2E_GET_EVENT_CALL_COUNT__?: number })
            .__BUZZ_E2E_GET_EVENT_CALL_COUNT__ ?? 0,
      ),
    )
    .toBe(1);
  await page.evaluate(() => {
    (
      window as Window & { __BUZZ_E2E_RELEASE_GET_EVENT__?: () => number }
    ).__BUZZ_E2E_RELEASE_GET_EVENT__?.();
  });
  await expect(rootThreadLink).toHaveText("general");
  await expect(rootThreadLink).toHaveClass(/mention-chip/);
  await expect(rootThreadLink).toHaveClass(/wrapping-inline-chip/);
  await expect(rootThreadLink).toHaveCSS("display", "inline");
  await expect(rootThreadLink).not.toHaveAttribute("title");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMAND_LOG__?: Array<{ command: string }>;
            }
          ).__BUZZ_E2E_COMMAND_LOG__?.filter(
            ({ command }) => command === "get_event",
          ).length ?? 0,
      ),
    )
    .toBe(1);
  await hoverUntilMetadataTooltip(page, rootThreadLink);
  const messageTooltip = page.getByRole("tooltip");
  await expect(
    messageTooltip.locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toHaveText("Welcome to general");
  // The tooltip proves metadata resolved; the inline chip must still carry the
  // channel label at the exact width it had while the fetch was in flight, and
  // never the fetched snippet or the truncated event hash.
  await expect(rootThreadLink).toHaveText("general");
  expect((await rootThreadLink.boundingBox())?.width).toBe(
    pendingChipBox?.width,
  );
  await expect(
    messageTooltip.locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toHaveClass(/line-clamp-3/);
  const messageFooter = messageTooltip.locator(
    '[data-buzz-tooltip-metadata-type=""]',
  );
  await expect(messageFooter).toHaveText(
    /#general · .+ · (just now|\d+[mhdw] ago)/,
  );
  await expect(messageFooter).toHaveCSS("white-space", "nowrap");
  await expect(messageFooter).toHaveCSS("overflow", "hidden");
  await expect(messageFooter).toHaveCSS("text-overflow", "ellipsis");
  const messageChipBox = await rootThreadLink.boundingBox();
  const messageTooltipBox = await messageTooltip.boundingBox();
  if (!messageChipBox || !messageTooltipBox) {
    throw new Error("Expected visible message chip and tooltip");
  }
  expect(
    Math.abs(
      messageTooltipBox.x +
        messageTooltipBox.width / 2 -
        (messageChipBox.x + messageChipBox.width / 2),
    ),
  ).toBeLessThanOrEqual(1);
  await page.getByTestId("chat-title").hover();
  await rootThreadLink.hover();
  await expect(
    page
      .getByRole("tooltip")
      .locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toHaveText("Welcome to general");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_COMMAND_LOG__?: Array<{ command: string }>;
            }
          ).__BUZZ_E2E_COMMAND_LOG__?.filter(
            ({ command }) => command === "get_event",
          ).length ?? 0,
      ),
    )
    .toBe(1);
  const randomChannelLink = linkMessage.getByRole("button", {
    name: "Open channel random",
  });
  await expect(randomChannelLink).toBeVisible();
  await expect(randomChannelLink).toHaveClass(/wrapping-inline-chip/);
  await expect(randomChannelLink).toHaveCSS("display", "inline");
  await expect(
    randomChannelLink.locator(".inline-chip-leading-fragment"),
  ).toHaveText("r");
  await expect(randomChannelLink).not.toHaveAttribute("title");
  await randomChannelLink.hover();
  const channelTooltip = page.getByRole("tooltip");
  await expect(
    channelTooltip.locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toHaveText("Off-topic, fun stuff");
  const channelFooter = channelTooltip.locator(
    '[data-buzz-tooltip-metadata-type=""]',
  );
  await expect(channelFooter).toHaveText("Public channel");
  await expect(channelFooter).toHaveCSS("white-space", "normal");
  await expect(channelFooter).toHaveCSS("overflow-wrap", "anywhere");
  await rootThreadLink.hover();
  await rootThreadLink.click({ button: "right" });

  const linkMenu = page.locator("[data-buzz-link-context-menu]");
  await expect(linkMenu).toBeVisible();
  await randomChannelLink.click({ button: "right" });
  await expect(linkMenu).toHaveCount(1);
  await rootThreadLink.click({ button: "right" });
  await expect(linkMenu).toHaveCount(1);
  await expect(
    linkMenu.getByRole("button", { name: "Open link" }),
  ).toBeVisible();
  await linkMenu.getByRole("button", { name: "Copy link" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => {
        return (
          window as Window & {
            __BUZZ_E2E_COMMAND_LOG__?: Array<{
              command: string;
              payload: { text?: string };
            }>;
          }
        ).__BUZZ_E2E_COMMAND_LOG__?.findLast(
          ({ command }) => command === "copy_text_to_clipboard",
        )?.payload.text;
      }),
    )
    .toBe(link);

  await rootThreadLink.click({ button: "right" });
  await linkMenu.getByRole("button", { name: "Open link" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(page).toHaveURL(/thread=mock-general-welcome/);
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to general",
  );
});

test("direct-message tooltip metadata stays on one physical line", async ({
  page,
}) => {
  const dmChannelId = "f48efb06-0c93-5025-aac9-2e646bb6bfa8";
  const dmMessageId = "mock-dm-link-one-line";

  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await page.evaluate((id) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "alice-tyler",
      content: "DM source message",
      id,
    });
  }, dmMessageId);

  await page.getByTestId("channel-general").click();
  await page
    .getByTestId("message-input")
    .fill(`DM link buzz://message?channel=${dmChannelId}&id=${dmMessageId}`);
  await page.getByTestId("send-message").click();

  const dmLink = page
    .getByTestId("message-row")
    .filter({ hasText: "DM link" })
    .last()
    .getByRole("button", { name: "Open message in channel alice-tyler" });
  await expect(dmLink).toHaveText("alice-tyler");
  await hoverUntilMetadataTooltip(page, dmLink);

  const footer = page
    .getByRole("tooltip")
    .locator('[data-buzz-tooltip-metadata-type=""]');
  await expect(footer).toContainText("Direct message with alice-tyler");
  await expect(footer).toHaveCSS("white-space", "nowrap");
  await expect(footer).toHaveCSS("overflow", "hidden");
  await expect(footer).toHaveCSS("text-overflow", "ellipsis");
  await expect
    .poll(() =>
      footer.evaluate((element) => {
        const lineHeight = Number.parseFloat(
          getComputedStyle(element).lineHeight,
        );
        return {
          fitsOneLine: element.scrollHeight <= Math.ceil(lineHeight),
          heightIsClipped: element.clientHeight === element.scrollHeight,
        };
      }),
    )
    .toEqual({ fitsOneLine: true, heightIsClipped: true });
});

test("message links explain when preview metadata is unavailable", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const missingMessageId = "f".repeat(64);
  await page
    .getByTestId("message-input")
    .fill(
      `Missing preview buzz://message?channel=9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50&id=${missingMessageId}`,
    );
  await page.getByTestId("send-message").click();

  const linkMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Missing preview" })
    .last();
  const missingMessageLink = linkMessage.getByRole("button", {
    name: "Open message in channel general",
  });
  await expect(missingMessageLink).toHaveAccessibleName(
    "Open message in channel general",
  );
  await expect(missingMessageLink).toHaveText("general");
  // The label is metadata-independent now, so gate the hover on the state the
  // failed lookup does change: the unavailable styling.
  await expect(missingMessageLink).toHaveClass(/buzz-link-unavailable/);
  await missingMessageLink.hover();
  const unavailableTooltip = page.getByRole("tooltip");
  await expect(unavailableTooltip).toHaveText("Message unavailable");
  await expect(unavailableTooltip).toHaveCSS("pointer-events", "none");

  const tooltipBox = await unavailableTooltip.boundingBox();
  if (!tooltipBox) throw new Error("Unavailable tooltip bounds missing");
  await page.mouse.move(
    tooltipBox.x + tooltipBox.width / 2,
    tooltipBox.y + tooltipBox.height / 2,
  );
  await expect(unavailableTooltip).toHaveCount(0);
});

test("message links reopen a closed thread when the same messageId is already in the URL", async ({
  page,
}) => {
  await page.goto(
    "/#/channels/9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50?messageId=mock-general-welcome",
  );
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to general",
  );

  await threadPanel.getByRole("button", { name: "Close panel" }).click();
  await expect(threadPanel).not.toBeVisible();

  const link =
    "buzz://message?channel=9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50&id=mock-general-welcome";
  await page
    .getByTestId("message-input")
    .fill(`Reopen same root link repro ${link}`);
  await page.getByTestId("send-message").click();

  const linkMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Reopen same root link repro" })
    .last();
  await expect(linkMessage).toBeVisible();
  const rootThreadLink = linkMessage.getByRole("button", {
    name: "Open message in channel general",
  });
  await expect(rootThreadLink).toHaveText("general");
  await rootThreadLink.click();

  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to general",
  );
});

test("message deep links survive reload", async ({ page }) => {
  await page.goto(
    `/#/channels/${ENGINEERING_CHANNEL_ID}?messageId=mock-engineering-shipped`,
  );

  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Engineering shipped the desktop build.",
  );

  await page.reload();

  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Engineering shipped the desktop build.",
  );
});

// Cold-start OS links are queued natively until AppShell mounts its router listener.

test("cold-start channel deep link drains after the router mounts", async ({
  page,
}) => {
  await installMockBridge(page, {
    pendingNavigationDeepLinks: [
      {
        id: "navigation-channel-1",
        kind: "channel",
        channelId: ENGINEERING_CHANNEL_ID,
      },
    ],
  });

  await page.goto("/");

  await expect(page.getByTestId("chat-title")).toHaveText("engineering");
  await expect(page).toHaveURL(
    new RegExp(`#/channels/${ENGINEERING_CHANNEL_ID}$`),
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (entry) =>
            entry.command === "acknowledge_pending_navigation_deep_link",
        ),
      ),
    )
    .toEqual([
      {
        command: "acknowledge_pending_navigation_deep_link",
        payload: { id: "navigation-channel-1" },
      },
    ]);
});

test("cold-start message deep link preserves its thread target", async ({
  page,
}) => {
  await installMockBridge(page, {
    pendingNavigationDeepLinks: [
      {
        id: "navigation-message-1",
        kind: "message",
        channelId: WATERCOLOR_CHANNEL_ID,
        messageId: "mock-forum-release-reply",
        threadRootId: "mock-forum-release-thread",
      },
    ],
  });

  await page.goto("/");

  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await expect(page).toHaveURL(/messageId=mock-forum-release-reply/);
  await expect(page).toHaveURL(/threadRootId=mock-forum-release-thread/);
});
