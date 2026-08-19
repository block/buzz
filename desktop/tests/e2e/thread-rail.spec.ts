import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

async function emitThread(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  parentEventId?: string,
  pubkey?: string,
) {
  return page.evaluate(
    ({ channelName: name, message, parentId, pubkey: authorPubkey }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string;
            pubkey?: string;
          }) => { id: string } | null;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock message emitter unavailable");
      const event = emit({
        channelName: name,
        content: message,
        parentEventId: parentId,
        pubkey: authorPubkey,
      });
      if (!event) throw new Error("Mock message emission failed");
      return event;
    },
    { channelName, message: content, parentId: parentEventId, pubkey },
  );
}

test("pins a canonical thread, switches it from the collapsible rail, and unpins locally", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const root = await emitThread(page, "general", "Thread rail root");
  await emitThread(page, "general", "Thread rail reply", root.id);
  const summary = page.locator(`[data-thread-head-id="${root.id}"]`);
  await expect(summary).toBeVisible();
  await summary.click();

  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await panel.getByTestId("pin-thread-to-rail").click();

  const rail = page.getByTestId("thread-rail");
  await expect(rail).toBeVisible();
  await expect(rail).toHaveAttribute("aria-label", "Pinned threads");
  const paneGeometry = await page.evaluate(() => {
    const railElement = document.querySelector<HTMLElement>(
      '[data-testid="thread-rail"]',
    );
    const railColumnElement = document.querySelector<HTMLElement>(
      '[data-testid="thread-rail-column"]',
    );
    const contentElement = document.querySelector<HTMLElement>(
      "[data-buzz-content-surface]:not([data-buzz-content-unframed])",
    );
    if (!railElement || !railColumnElement || !contentElement) {
      throw new Error(
        "Thread Rail, rail column, or framed content surface unavailable",
      );
    }
    const railRect = railElement.getBoundingClientRect();
    const contentRect = contentElement.getBoundingClientRect();
    const railStyle = getComputedStyle(railElement);
    const railColumnStyle = getComputedStyle(railColumnElement);
    const contentStyle = getComputedStyle(contentElement);
    return {
      railTop: Math.round(railRect.top),
      contentTop: Math.round(contentRect.top),
      railBottom: Math.round(railRect.bottom),
      contentBottom: Math.round(contentRect.bottom),
      railRadius: railStyle.borderTopLeftRadius,
      contentRadius: contentStyle.borderTopLeftRadius,
      railBackground: railStyle.backgroundColor,
      railColumnBackgroundImage: railColumnStyle.backgroundImage,
      railColumnRadius: railColumnStyle.borderTopLeftRadius,
      contentBackground: contentStyle.backgroundColor,
      railClipPath: railStyle.clipPath,
    };
  });
  expect(paneGeometry).toMatchObject({
    railTop: paneGeometry.contentTop,
    railRadius: paneGeometry.contentRadius,
    railBackground: paneGeometry.contentBackground,
  });
  expect(paneGeometry.railBottom).toBeLessThan(paneGeometry.contentBottom);
  expect(paneGeometry.railColumnRadius).toBe("0px");
  expect(paneGeometry.railColumnBackgroundImage).toContain("linear-gradient");
  expect(paneGeometry.railClipPath).toBe("none");
  const entry = rail.getByTestId(`thread-rail-entry-${root.id}`);
  await expect(entry).toHaveAttribute("aria-current", "page");
  const visualContract = await page.evaluate((rootId) => {
    const railElement = document.querySelector<HTMLElement>(
      '[data-testid="thread-rail"]',
    );
    const headerElement = document.querySelector<HTMLElement>(
      '[data-testid="thread-rail-header"]',
    );
    const rowElement = document.querySelector<HTMLElement>(
      `[data-testid="thread-rail-row-${rootId}"]`,
    );
    const entryElement = document.querySelector<HTMLElement>(
      `[data-testid="thread-rail-entry-${rootId}"]`,
    );
    if (!railElement || !headerElement || !rowElement || !entryElement) {
      throw new Error("Thread Rail visual contract elements unavailable");
    }
    return {
      headerHeight: Math.round(headerElement.getBoundingClientRect().height),
      headerBorderBottomWidth:
        getComputedStyle(headerElement).borderBottomWidth,
      rowInset: Math.round(
        railElement.getBoundingClientRect().width -
          rowElement.getBoundingClientRect().width,
      ),
      rowBackground: getComputedStyle(rowElement).backgroundColor,
      entryBackground: getComputedStyle(entryElement).backgroundColor,
    };
  }, root.id);
  expect(visualContract.headerHeight).toBeGreaterThanOrEqual(52);
  expect(visualContract.headerBorderBottomWidth).toBe("0px");
  expect(visualContract.rowInset).toBe(16);
  expect(visualContract.rowBackground).not.toBe("rgba(0, 0, 0, 0)");
  expect(visualContract.entryBackground).toBe("rgba(0, 0, 0, 0)");

  const expandedWidth = await rail.evaluate((element) =>
    Math.round(element.getBoundingClientRect().width),
  );
  await rail.getByTestId("thread-rail-toggle").click();
  await expect(rail).toHaveAttribute("data-collapsed", "true");
  await expect(entry).toBeHidden();
  await expect(rail).toHaveAttribute("data-collapsed", "true");
  await expect
    .poll(() =>
      rail.evaluate((element) =>
        Math.round(element.getBoundingClientRect().width),
      ),
    )
    .toBeLessThan(expandedWidth);
  await rail.getByTestId("thread-rail-toggle").click();
  await expect(entry).toBeVisible();

  await page.getByTestId("channel-random").click();
  await entry.click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("message-thread-head")).toContainText(
    "Thread rail root",
  );

  await rail.getByTestId(`unpin-thread-rail-${root.id}`).click();
  await expect(rail).toBeHidden();
  await expect(panel).toBeVisible();
  await expect(panel.getByTestId("message-input")).toBeVisible();
  const storedRail = await page.evaluate(() =>
    Object.keys(localStorage)
      .filter((key) => key.startsWith("buzz-thread-rail.v1:"))
      .map((key) => localStorage.getItem(key)),
  );
  await expect(storedRail).toContain(
    '{"version":1,"collapsed":false,"pins":[]}',
  );
});

test("shows an unread dot for a pinned thread with a new reply", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const root = await emitThread(page, "general", "Pinned thread root");
  await emitThread(page, "general", "Pinned thread reply", root.id);
  await page.locator(`[data-thread-head-id="${root.id}"]`).click();

  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await panel.getByTestId("pin-thread-to-rail").click();

  const rail = page.getByTestId("thread-rail");
  const entry = rail.getByTestId(`thread-rail-entry-${root.id}`);
  await expect(entry).toBeVisible();

  await page.getByTestId("channel-random").click();
  await emitThread(
    page,
    "general",
    "Unread reply in pinned thread",
    root.id,
    TEST_IDENTITIES.alice.pubkey,
  );

  await expect(rail.getByTestId(`thread-rail-unread-${root.id}`)).toBeVisible();
});

test("returns to a pinned nested reply with its ancestors expanded", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const root = await emitThread(page, "general", "Return anchor root");
  const parent = await emitThread(
    page,
    "general",
    "Return anchor parent",
    root.id,
  );
  const nested = await emitThread(
    page,
    "general",
    "Return anchor nested",
    parent.id,
  );
  await page.locator(`[data-thread-head-id="${root.id}"]`).click();

  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await panel.locator(`[data-thread-head-id="${parent.id}"]`).click();
  await expect(panel.getByText("Return anchor nested")).toBeVisible();
  await panel.getByTestId(`reply-message-${nested.id}`).dispatchEvent("click");
  await expect(panel.getByTestId("reply-target")).toContainText(
    "Return anchor nested",
  );
  await panel.getByTestId("pin-thread-to-rail").click();

  const rail = page.getByTestId("thread-rail");
  const entry = rail.getByTestId(`thread-rail-entry-${root.id}`);
  await expect(entry).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        Object.keys(localStorage)
          .filter((key) => key.startsWith("buzz-thread-rail.v1:"))
          .map((key) => localStorage.getItem(key))
          .join(""),
      ),
    )
    .toContain(`"returnAnchorId":"${nested.id}"`);

  await panel.getByTestId(`reply-message-${parent.id}`).dispatchEvent("click");
  await expect
    .poll(() =>
      page.evaluate(() =>
        Object.keys(localStorage)
          .filter((key) => key.startsWith("buzz-thread-rail.v1:"))
          .map((key) => localStorage.getItem(key))
          .join(""),
      ),
    )
    .toContain(`"returnAnchorId":"${parent.id}"`);
  await panel.getByTestId(`reply-message-${nested.id}`).dispatchEvent("click");
  await expect
    .poll(() =>
      page.evaluate(() =>
        Object.keys(localStorage)
          .filter((key) => key.startsWith("buzz-thread-rail.v1:"))
          .map((key) => localStorage.getItem(key))
          .join(""),
      ),
    )
    .toContain(`"returnAnchorId":"${nested.id}"`);

  await page.getByTestId("channel-random").click();
  await entry.click();

  await expect(panel.getByTestId("message-thread-head")).toContainText(
    "Return anchor root",
  );
  await expect(panel.getByText("Return anchor parent")).toBeVisible();
  await expect(panel.getByText("Return anchor nested")).toBeVisible();
  await expect(panel.getByTestId("reply-target")).toHaveCount(0);
});

test("falls back to a pinned root after an unavailable return anchor settles", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await emitThread(
    page,
    "general",
    "Temporary reply used to pin the default root",
    "mock-general-welcome",
  );
  await page.locator('[data-thread-head-id="mock-general-welcome"]').click();
  await page
    .getByTestId("message-thread-panel")
    .getByTestId("pin-thread-to-rail")
    .click();

  await page.evaluate(() => {
    const key = Object.keys(localStorage).find((candidate) =>
      candidate.startsWith("buzz-thread-rail.v1:"),
    );
    if (!key) throw new Error("Thread Rail store was not created");
    const value = JSON.parse(localStorage.getItem(key) ?? "null");
    value.pins[0].returnAnchorId = "f".repeat(64);
    localStorage.setItem(key, JSON.stringify(value));
  });
  await page.reload();

  await expect(page.getByTestId("thread-rail")).toBeVisible();
  await page.getByTestId("channel-random").click();
  await page.getByTestId("thread-rail-entry-mock-general-welcome").click();
  const panel = page.getByTestId("message-thread-panel");
  await expect(panel.getByTestId("message-thread-head")).toContainText(
    "Welcome to general",
  );
  await expect(panel.getByTestId("reply-target")).toHaveCount(0);
});

test("keeps every manually expanded pinned-thread branch open after switching away and back", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const root = await emitThread(page, "general", "Expansion memory root");
  const branchA = await emitThread(
    page,
    "general",
    "Expansion branch A",
    root.id,
  );
  await emitThread(page, "general", "Expansion leaf A", branchA.id);
  const branchB = await emitThread(
    page,
    "general",
    "Expansion branch B",
    root.id,
  );
  await emitThread(page, "general", "Expansion leaf B", branchB.id);
  await page.locator(`[data-thread-head-id="${root.id}"]`).click();

  const panel = page.getByTestId("message-thread-panel");
  await panel.locator(`[data-thread-head-id="${branchA.id}"]`).click();
  await expect(panel.getByText("Expansion leaf A")).toBeVisible();
  await panel.locator(`[data-thread-head-id="${branchB.id}"]`).click();
  await expect(panel.getByText("Expansion leaf B")).toBeVisible();
  await panel.getByTestId("pin-thread-to-rail").click();

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  const otherRoot = await emitThread(page, "random", "Other pinned root");
  await emitThread(page, "random", "Other pinned reply", otherRoot.id);
  const otherSummary = page.locator(`[data-thread-head-id="${otherRoot.id}"]`);
  await expect(otherSummary).toBeVisible();
  await otherSummary.click();
  await panel.getByTestId("pin-thread-to-rail").click();

  await page.getByTestId(`thread-rail-entry-${root.id}`).click();
  await expect(panel.getByText("Expansion leaf A")).toBeVisible();
  await expect(panel.getByText("Expansion leaf B")).toBeVisible();
  await expect(panel.getByTestId("reply-target")).toHaveCount(0);
});
