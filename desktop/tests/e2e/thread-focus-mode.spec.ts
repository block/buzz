import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function seedLongThread(page: import("@playwright/test").Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  return page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Focus mode integration thread",
      createdAt: 1_700_900_000,
    });
    if (!root) throw new Error("Failed to seed focus thread root");

    for (let index = 0; index < 48; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `Focus reply ${index}: this deliberately wraps across several lines so changing the thread measure causes real layout reflow.`,
        parentEventId: root.id,
        createdAt: 1_700_900_001 + index,
      });
    }
    return root.id;
  });
}

async function topVisibleMessageId(
  body: import("@playwright/test").Locator,
): Promise<string> {
  return body.evaluate((element) => {
    const top = element.getBoundingClientRect().top;
    const row = Array.from(
      element.querySelectorAll<HTMLElement>("[data-message-id]"),
    ).find((candidate) => candidate.getBoundingClientRect().bottom > top);
    if (!row?.dataset.messageId) throw new Error("No visible thread anchor");
    return row.dataset.messageId;
  });
}

test("focus and split preserve reading context and interaction ownership", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.channels.threadViewMode", "focus");
  });
  await installMockBridge(page);
  await page.goto("/");
  const rootId = await seedLongThread(page);

  await page.getByTestId("channel-general").click();
  const summary = page.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible();
  await summary.click();

  const channel = page.getByTestId("channel-drop-zone");
  const drawer = page.getByTestId("focus-thread-drawer");
  const body = page.getByTestId("message-thread-body");
  await expect(drawer).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean(
          document
            .querySelector('[data-testid="focus-thread-drawer"]')
            ?.contains(document.activeElement),
        ),
      ),
    )
    .toBe(true);
  await expect(channel).toHaveAttribute("inert", "");

  await body.evaluate((element) => {
    element.scrollTop = element.scrollHeight * 0.4;
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  const anchorId = await topVisibleMessageId(body);

  await page
    .getByRole("button", { name: "Show thread beside channel" })
    .click();
  await expect(drawer).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");
  await expect(page.getByTestId("thread-view-mode-toggle")).toBeFocused();
  await expect(
    body.locator(`[data-message-id="${anchorId}"]`),
  ).toBeInViewport();
  await expect(
    body.locator(`[data-message-id="${anchorId}"]`),
  ).not.toHaveAttribute("data-highlighted", "true");

  await page.getByRole("button", { name: "Expand thread" }).click();
  await expect(drawer).toBeVisible();
  await expect(channel).toHaveAttribute("inert", "");
  await expect(page.getByTestId("thread-view-mode-toggle")).toBeFocused();
  await expect(
    body.locator(`[data-message-id="${anchorId}"]`),
  ).toBeInViewport();

  // Escape layering: a nested control inside the drawer claims Escape first.
  // With mention autocomplete open, Escape closes only the autocomplete — the
  // drawer must stay.
  const threadInput = page
    .getByTestId("message-thread-panel")
    .getByTestId("message-input");
  await threadInput.click();
  await threadInput.pressSequentially("@al");
  const mentionDropdown = page
    .getByTestId("message-thread-panel")
    .getByTestId("mention-autocomplete");
  await expect(mentionDropdown).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(mentionDropdown).toHaveCount(0);
  await expect(drawer).toBeVisible();

  // Known gap (pre-existing, tracked as a follow-up): while the composer's
  // rich-text editor holds focus it claims Escape internally even with no
  // autocomplete open, so Escape-from-composer never reaches the drawer's
  // close listener. Move focus to the drawer itself before asserting
  // Escape-to-close. When the editor is taught to release an idle Escape,
  // this focus hop can be removed.
  await drawer.focus();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("focus-thread-drawer-overlay")).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");

  await summary.click();
  await expect(drawer).toBeVisible();
  await page.getByTestId("focus-thread-drawer-scrim").click({
    position: { x: 24, y: 200 },
  });
  await expect(page.getByTestId("focus-thread-drawer-overlay")).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");
});

test("narrow threads do not offer an unavailable layout switch", async ({
  page,
}) => {
  await page.setViewportSize({ width: 860, height: 720 });
  await installMockBridge(page);
  await page.goto("/");
  const rootId = await seedLongThread(page);
  await page.getByTestId("channel-general").click();
  const summary = page.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible();
  await summary.click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await expect(page.getByTestId("thread-view-mode-toggle")).toHaveCount(0);
});
