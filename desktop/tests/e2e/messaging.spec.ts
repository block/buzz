import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("send a message and see it in timeline", async ({ page }) => {
  const message = `Hello timeline ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
  await expect(page.getByTestId("message-row").last()).toContainText(
    "npub1mock...",
  );
});

test("send multiple messages in sequence", async ({ page }) => {
  const ts = Date.now();
  const messages = [
    `First message ${ts}`,
    `Second message ${ts}`,
    `Third message ${ts}`,
  ];
  const input = page.getByTestId("message-input");
  const sendButton = page.getByTestId("send-message");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  for (const message of messages) {
    await input.fill(message);
    await sendButton.click();
    await expect(page.getByTestId("message-timeline")).toContainText(message);
  }

  const timeline = page.getByTestId("message-timeline");
  for (const message of messages) {
    await expect(timeline).toContainText(message);
  }
});

test("copy a rendered code block and paste it back as code", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  const code = "# not a heading\nconst answer = 42;\n  indented();";

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    `\`\`\`ts\n${code}\n\`\`\``,
  );
  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await page.getByTestId("send-message").click();

  const codeBlock = page.locator("[data-code-block]");
  await expect(codeBlock).toHaveCount(1);

  const copyButton = page.getByLabel("Copy code block");
  await expect(copyButton).toHaveCSS("opacity", "0");
  await codeBlock.hover();
  await expect(copyButton).toHaveCSS("opacity", "1");
  await copyButton.click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe(code);

  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await input.press("Enter");

  await expect(codeBlock).toHaveCount(2);
});

test("pasting a long copied code block scrolls composer to cursor", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  const longCode = Array.from(
    { length: 48 },
    (_, index) => `const line${index} = ${index};`,
  ).join("\n");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    `\`\`\`ts\n${longCode}\n\`\`\``,
  );
  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await page.getByTestId("send-message").click();

  const copiedCodeBlock = page.locator("[data-code-block]");
  await expect(copiedCodeBlock).toHaveCount(1);
  await copiedCodeBlock.hover();
  await page.getByLabel("Copy code block").click();

  await input.fill("typed before paste");
  await page.keyboard.press("ControlOrMeta+V");

  const scrollContainer = page.getByTestId("message-input-scroll");
  await expect
    .poll(() =>
      scrollContainer.evaluate(
        (element) =>
          element.scrollHeight - element.clientHeight - element.scrollTop,
      ),
    )
    .toBeLessThanOrEqual(1);
});

test("code block shows language label when language is specified", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    "```typescript\nconst x = 1;\n```",
  );

  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await page.getByTestId("send-message").click();

  const codeBlock = page.locator("[data-code-block]");
  await expect(codeBlock).toBeVisible();
  await expect(codeBlock.getByText("typescript")).toBeVisible();
});

test("typing triple backticks and Enter creates a code block in composer", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("```");
  await page.keyboard.press("Enter");

  // A <pre> code block should appear inside the ProseMirror editor
  const editorPre = input.locator("pre");
  await expect(editorPre).toBeVisible();

  // The literal backticks should be consumed (not visible as text)
  await expect(input).not.toContainText("```");
});

test("message input clears after send", async ({ page }) => {
  const message = `Clear after send ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(message);
  await expect(input).toHaveText(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
  await expect(input).toHaveText("");
});

test("emoji picker inserts emoji into the draft and keeps focus in the composer", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Ship");

  await page.getByTestId("composer-emoji-button").click();

  // emoji-mart renders inside a Shadow DOM web component — use the search
  // input to find the rocket emoji, then click it.
  const pickerEl = page.locator("em-emoji-picker");
  const searchInput = pickerEl.locator("input[type='search']");
  await searchInput.fill("rocket");
  await pickerEl.locator("button[aria-label='🚀']").first().click();

  await expect(input).toHaveText("Ship🚀");
  await expect(input).toBeFocused();

  await input.pressSequentially(" now");
  await expect(input).toHaveText("Ship🚀 now");
});

test("empty message cannot be sent", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const sendButton = page.getByTestId("send-message");
  await expect(sendButton).toBeDisabled();
});

test("send message with Enter key", async ({ page }) => {
  const message = `Enter key send ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(message);
  await input.press("Enter");

  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("messages persist across channel switches", async ({ page }) => {
  const message = `Persist across switch ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("draft is preserved when switching channels", async ({ page }) => {
  const draft = `Unsent draft ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Type a draft but do not send it
  await input.fill(draft);
  await expect(input).toHaveText(draft);

  // Switch to another channel — composer should be empty
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(input).toHaveText("");

  // Switch back — the draft should still be there
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(input).toHaveText(draft);
});

test("sending a message clears the draft", async ({ page }) => {
  const message = `Sent message ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Type and send a message
  await input.fill(message);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  // Switch away and back — composer should be empty, not restored from draft
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(input).toHaveText("");
});

test("different channels have independent messages", async ({ page }) => {
  const ts = Date.now();
  const generalMessage = `General only ${ts}`;
  const randomMessage = `Random only ${ts}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("message-input").fill(generalMessage);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(
    generalMessage,
  );

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    generalMessage,
  );

  await page.getByTestId("message-input").fill(randomMessage);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(
    randomMessage,
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    generalMessage,
  );
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    randomMessage,
  );
});

test("day divider appears in timeline", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );
  await expect(page.getByTestId("message-timeline-day-divider")).toBeVisible();
});

test("send message to DM channel", async ({ page }) => {
  const message = `DM message ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("shows your avatar on your own message when profile avatar is set", async ({
  page,
}) => {
  const message = `Avatar message ${Date.now()}`;
  const avatarUrl =
    'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"%3E%3Crect width="16" height="16" rx="4" fill="%2300a36c"/%3E%3C/svg%3E';

  await page.goto("/");
  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();
  await page.getByTestId("profile-avatar-url").fill(avatarUrl);
  await page.getByTestId("profile-avatar-done").click();
  await page.getByTestId("settings-back-to-app").click();

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  const lastMessage = page.getByTestId("message-row").last();
  await expect(lastMessage).toContainText(message);
  await expect(lastMessage.getByTestId("message-avatar-image")).toHaveAttribute(
    "src",
    avatarUrl,
  );
});

test("opens a single-level thread panel with inline expansion", async ({
  page,
}) => {
  const timestamp = Date.now();
  const firstReply = `First threaded reply ${timestamp}`;
  const siblingReply = `Sibling threaded reply ${timestamp}`;
  const nestedReply = `Nested threaded reply ${timestamp}`;
  const nestedReplyFromBob = `Nested reply from Bob ${timestamp}`;
  const nestedReplyVisibleTopMaxPx = 280;
  const fillerReplies = Array.from(
    { length: 14 },
    (_, index) => `Thread filler reply ${index} ${timestamp}`,
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to #general",
  );

  const timeline = page.getByTestId("message-timeline");
  const timelineRows = timeline.getByTestId("message-row");
  const threadPanel = page.getByTestId("message-thread-panel");
  const threadBody = threadPanel.getByTestId("message-thread-body");
  const threadComposer = threadPanel.locator('[data-testid="message-input"]');
  const threadSendButton = threadPanel.getByTestId("send-message");
  const threadReplies = threadPanel.getByTestId("message-thread-replies");
  const rootMessage = timelineRows.first();
  const rootMessageId = await rootMessage.getAttribute("data-message-id");
  if (!rootMessageId) {
    throw new Error("Expected root message row to have a data-message-id.");
  }
  const rootSummaryRow = timeline.locator(
    `[data-thread-head-id="${rootMessageId}"]`,
  );

  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to #general",
  );

  await threadComposer.fill(firstReply);
  await threadSendButton.click();
  await expect(threadReplies).toContainText(firstReply);

  await threadComposer.fill(siblingReply);
  await threadSendButton.click();
  await expect(threadReplies).toContainText(siblingReply);

  for (const fillerReply of fillerReplies) {
    await threadComposer.fill(fillerReply);
    await threadSendButton.click();
    await expect(threadReplies).toContainText(fillerReply);
  }

  await expect
    .poll(async () => {
      return threadBody.evaluate((element) => {
        const body = element as HTMLDivElement;
        return body.scrollHeight - body.clientHeight;
      });
    })
    .toBeGreaterThanOrEqual(160);

  await expect(
    timeline.getByTestId("message-row").filter({ hasText: firstReply }),
  ).toHaveCount(0);
  await expect(
    timeline.getByTestId("message-row").filter({ hasText: siblingReply }),
  ).toHaveCount(0);

  await expect(rootSummaryRow).toContainText("16 replies");
  await expect(
    rootSummaryRow.getByTestId("message-thread-summary-participant"),
  ).toHaveCount(1);
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-participant")
        .first()
        .evaluate((wrapper) => {
          const avatar = wrapper.firstElementChild;
          if (!(avatar instanceof HTMLElement)) return "missing";
          const rect = avatar.getBoundingClientRect();
          return `${Math.round(rect.width)}x${Math.round(rect.height)}`;
        }),
    )
    .toBe("28x28");

  await page.mouse.move(0, 0);
  const rootSummaryWidthBeforeHover = await rootSummaryRow.evaluate((row) =>
    Math.round(row.getBoundingClientRect().width),
  );
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-last-reply")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeGreaterThan(0.8);
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-hover-action")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeLessThan(0.1);
  await rootSummaryRow.hover();
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-last-reply")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeLessThan(0.1);
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-hover-action")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeGreaterThan(0.8);
  await expect
    .poll(() =>
      rootSummaryRow.evaluate((row) =>
        Math.round(row.getBoundingClientRect().width),
      ),
    )
    .toBe(rootSummaryWidthBeforeHover);

  await threadPanel.getByTestId("message-thread-close").click();
  await expect(threadPanel).toBeHidden();

  await rootSummaryRow.click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to #general",
  );

  const firstReplyRow = threadReplies
    .getByTestId("message-row")
    .filter({ hasText: firstReply })
    .first();
  await firstReplyRow.hover();
  await firstReplyRow.getByRole("button", { name: "Reply" }).click();

  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to #general",
  );
  await expect(threadPanel.getByTestId("message-thread-back")).toHaveCount(0);

  await threadComposer.fill(nestedReply);
  await threadSendButton.click();

  const nestedReplyRow = threadReplies
    .getByTestId("message-row")
    .filter({ hasText: nestedReply })
    .first();
  await expect(nestedReplyRow).toBeVisible();
  await expect(
    timeline.getByTestId("message-row").filter({ hasText: nestedReply }),
  ).toHaveCount(0);

  await expect(
    threadReplies.getByTestId("message-row").filter({ hasText: siblingReply }),
  ).toHaveCount(1);
  await expect
    .poll(async () => {
      return nestedReplyRow.evaluate((row) => {
        const threadBody = row.closest(
          '[data-testid="message-thread-body"]',
        ) as HTMLElement | null;
        if (!threadBody) {
          return Number.POSITIVE_INFINITY;
        }

        const rowRect = row.getBoundingClientRect();
        const bodyRect = threadBody.getBoundingClientRect();
        return rowRect.top - bodyRect.top;
      });
    })
    .toBeLessThanOrEqual(nestedReplyVisibleTopMaxPx);

  const firstReplyId = await firstReplyRow.getAttribute("data-message-id");
  if (!firstReplyId) {
    throw new Error("Expected first reply row to have a data-message-id.");
  }

  await page.evaluate(
    ({ content, parentEventId, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        parentEventId,
        pubkey,
      });
    },
    {
      content: nestedReplyFromBob,
      parentEventId: firstReplyId,
      pubkey: TEST_IDENTITIES.bob.pubkey,
    },
  );
  const nestedReplyFromBobRow = threadReplies
    .getByTestId("message-row")
    .filter({ hasText: nestedReplyFromBob })
    .first();
  await expect(nestedReplyFromBobRow).toBeVisible();

  const firstReplySummaryRow = threadReplies.locator(
    `[data-thread-head-id="${firstReplyId}"]`,
  );
  await expect(firstReplySummaryRow).toHaveCount(1);
  await expect(firstReplySummaryRow).toContainText("2 replies");

  await expect(rootSummaryRow).toContainText("18 replies");
  await expect(
    rootSummaryRow.getByTestId("message-thread-summary-participant"),
  ).toHaveCount(2);

  await expect
    .poll(async () => {
      return nestedReplyRow.evaluate((row) => {
        const threadBody = row.closest(
          '[data-testid="message-thread-body"]',
        ) as HTMLElement | null;
        if (!threadBody) {
          return Number.POSITIVE_INFINITY;
        }

        const rowRect = row.getBoundingClientRect();
        const bodyRect = threadBody.getBoundingClientRect();
        return rowRect.top - bodyRect.top;
      });
    })
    .toBeLessThanOrEqual(nestedReplyVisibleTopMaxPx);

  await firstReplySummaryRow.click();
  await expect(
    threadReplies.getByTestId("message-row").filter({ hasText: nestedReply }),
  ).toHaveCount(0);
  await expect(
    threadReplies
      .getByTestId("message-row")
      .filter({ hasText: nestedReplyFromBob }),
  ).toHaveCount(0);
});

test("thread panel width uses session storage and reset handle", async ({
  page,
}) => {
  const customWidthPx = 520;
  const defaultWidthPx = 380;

  await page.addInitScript((width) => {
    window.sessionStorage.setItem(
      "buzz.desktop.thread-panel-width",
      String(width),
    );
  }, customWidthPx);

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  const rootMessage = timeline.getByTestId("message-row").first();
  const threadPanel = page.getByTestId("message-thread-panel");
  const resizeHandle = threadPanel.getByTestId(
    "right-auxiliary-pane-resize-handle",
  );

  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(threadPanel).toBeVisible();

  await expect
    .poll(async () => {
      return threadPanel.evaluate((panel) => {
        const element = panel as HTMLElement;
        return Math.round(element.getBoundingClientRect().width);
      });
    })
    .toBe(customWidthPx);

  await resizeHandle.dblclick();

  await expect
    .poll(async () => {
      return threadPanel.evaluate((panel) => {
        const element = panel as HTMLElement;
        return Math.round(element.getBoundingClientRect().width);
      });
    })
    .toBe(defaultWidthPx);

  await threadPanel.getByTestId("message-thread-close").click();
  await expect(threadPanel).toBeHidden();

  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(threadPanel).toBeVisible();

  await expect
    .poll(async () => {
      return threadPanel.evaluate((panel) => {
        const element = panel as HTMLElement;
        return Math.round(element.getBoundingClientRect().width);
      });
    })
    .toBe(defaultWidthPx);
});

test("narrow thread view collapses channel header actions into a menu", async ({
  page,
}) => {
  await page.setViewportSize({ width: 980, height: 720 });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("channel-add-bot-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-actions-menu-trigger")).toHaveCount(0);

  const rootMessage = page.locator('[data-message-id="mock-general-alice"]');
  const threadPanel = page.getByTestId("message-thread-panel");

  await rootMessage.hover();
  await page.getByTestId("reply-message-mock-general-alice").click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-back")).toHaveCount(0);

  const menuTrigger = page.getByTestId("channel-actions-menu-trigger");
  await expect(menuTrigger).toBeVisible();
  await expect(page.getByTestId("channel-add-bot-trigger")).toBeHidden();
  await expect(page.getByTestId("channel-members-trigger")).toBeHidden();
  await expect(page.getByTestId("channel-management-trigger")).toBeHidden();

  const menuBox = await menuTrigger.boundingBox();
  const threadPanelBox = await threadPanel.boundingBox();
  if (!menuBox || !threadPanelBox) {
    throw new Error("Expected header action menu and thread panel bounds");
  }
  const menuGapPx = threadPanelBox.x - (menuBox.x + menuBox.width);
  expect(menuGapPx).toBeGreaterThanOrEqual(22);
  expect(menuGapPx).toBeLessThanOrEqual(26);

  await menuTrigger.click();

  await expect(page.getByTestId("channel-add-bot-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-members-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-start-huddle-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-management-trigger")).toBeVisible();
});

test("single-panel thread view hides topbar search and channel actions", async ({
  page,
}) => {
  await page.setViewportSize({ width: 860, height: 720 });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("open-search")).toBeVisible();
  await expect(page.getByTestId("channel-add-bot-trigger")).toBeVisible();

  const rootMessage = page.locator('[data-message-id="mock-general-alice"]');
  const threadPanel = page.getByTestId("message-thread-panel");

  await rootMessage.hover();
  await page.getByTestId("reply-message-mock-general-alice").click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-back")).toBeVisible();
  await expect(page.getByTestId("open-search")).toHaveCount(0);
  await expect(page.getByTestId("channel-actions-menu-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);

  await threadPanel.getByTestId("message-thread-back").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("open-search")).toBeVisible();
  await expect(page.getByTestId("channel-add-bot-trigger")).toBeVisible();
});

test("composer is focused after selecting a channel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Without clicking the input, typing should land in the composer.
  const input = page.getByTestId("message-input");
  await expect(input).toBeFocused();

  await page.keyboard.type("autofocus-on-channel-select");
  await expect(input).toHaveText("autofocus-on-channel-select");
});

test("composer is focused after switching to a different channel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  const input = page.getByTestId("message-input");
  await expect(input).toBeFocused();
});

test("thread composer is focused after clicking the reply icon", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Seed a message to reply to.
  const seed = `Thread autofocus seed ${Date.now()}`;
  const mainInput = page.getByTestId("message-input");
  await mainInput.fill(seed);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(seed);

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  const threadInput = threadPanel.getByTestId("message-input");
  await expect(threadInput).toBeFocused();

  await page.keyboard.type("typed-into-thread");
  await expect(threadInput).toHaveText("typed-into-thread");
});

test("thread composer keeps focus after sending a thread reply", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Seed a root message we can open a thread on. At this point only one
  // composer is mounted, so plain getByTestId is unambiguous.
  const seed = `Thread focus-after-send seed ${Date.now()}`;
  await page.getByTestId("message-input").fill(seed);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(seed);

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  const threadInput = threadPanel.getByTestId("message-input");
  await expect(threadInput).toBeFocused();

  // Send a thread reply. After the send, `isSending` flips and back to false
  // in both the main and thread composers; the thread input must keep focus.
  const reply = `Thread reply ${Date.now()}`;
  await page.keyboard.type(reply);
  await expect(threadInput).toHaveText(reply);
  await page.keyboard.press("Enter");

  // Wait for the send to settle.
  await expect(threadPanel).toContainText(reply);

  // The thread input should still be focused — not the main composer.
  // Both composers expose the same `message-input` data-testid, so we
  // verify directly that `document.activeElement` lives inside the thread
  // panel rather than the main pane.
  const focusInThreadPanel = await page.evaluate(() => {
    const panel = document.querySelector<HTMLElement>(
      '[data-testid="message-thread-panel"]',
    );
    const active = document.activeElement as HTMLElement | null;
    return Boolean(panel && active && panel.contains(active));
  });
  expect(focusInThreadPanel).toBe(true);

  await expect(threadInput).toBeFocused();
});

test("ArrowUp in an empty composer edits your last message right after sending", async ({
  page,
}) => {
  // Regression: after a send, the composer keeps DOM focus and ProseMirror
  // would consume ArrowUp before it reached the edit-last-message handler,
  // so ↑ did nothing until you clicked out and back. The handler now lives
  // in the editor keymap, so ↑ must work with no intermediate click.
  const message = `Edit-last via arrow up ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(message);
  await input.press("Enter");
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  // Composer stays focused after send — no click, just press ↑.
  await expect(input).toBeFocused();
  await page.keyboard.press("ArrowUp");

  // Edit mode is entered for the just-sent message.
  const editBanner = page.getByTestId("edit-target");
  await expect(editBanner).toBeVisible();
  await expect(editBanner).toContainText(message);
  await expect(input).toHaveText(message);
});

test("ArrowUp does not edit when the composer has draft text", async ({
  page,
}) => {
  // Guard: ↑ must only hijack to edit when the composer is empty, so it
  // never steals the arrow key from someone navigating drafted text.
  const sent = `Sent before draft ${Date.now()}`;
  const draft = `Half-typed draft ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(sent);
  await input.press("Enter");
  await expect(page.getByTestId("message-timeline")).toContainText(sent);

  await input.fill(draft);
  await expect(input).toHaveText(draft);
  await page.keyboard.press("ArrowUp");

  // No edit mode; the draft is untouched.
  await expect(page.getByTestId("edit-target")).toHaveCount(0);
  await expect(input).toHaveText(draft);
});

test("ArrowUp edits your last thread reply right after sending it", async ({
  page,
}) => {
  // Same fix must hold in the thread composer (shares MessageComposer).
  const seed = `Thread arrow-up seed ${Date.now()}`;
  const reply = `Thread reply to edit ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(seed);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(seed);

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  const threadInput = threadPanel.getByTestId("message-input");
  await expect(threadInput).toBeFocused();

  await page.keyboard.type(reply);
  await page.keyboard.press("Enter");
  await expect(threadPanel).toContainText(reply);

  // No click — press ↑ in the still-focused thread composer.
  await page.keyboard.press("ArrowUp");

  const editBanner = threadPanel.getByTestId("edit-target");
  await expect(editBanner).toBeVisible();
  await expect(editBanner).toContainText(reply);
  await expect(threadInput).toHaveText(reply);
});
