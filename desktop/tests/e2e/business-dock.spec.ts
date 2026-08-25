import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

async function openGeneralThread(page: import("@playwright/test").Page) {
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
          }) ?? false,
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ pubkey }) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId: string;
            pubkey: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: "Business Dock coexistence reply",
        parentEventId: "mock-general-welcome",
        pubkey,
      }),
    { pubkey: TEST_IDENTITIES.alice.pubkey },
  );
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
}

async function emitBusinessMessage(
  page: import("@playwright/test").Page,
  content: string,
  pubkey = TEST_IDENTITIES.alice.pubkey,
) {
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect
    .poll(async () =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ message, author }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: message,
        pubkey: author,
        createdAt: Math.floor(Date.now() / 1000),
      }),
    { message: content, author: pubkey },
  );
}

test.describe("Business Dock", () => {
  test.use({ viewport: { width: 1600, height: 900 } });

  test.beforeEach(async ({ page }) => {
    await installMockBridge(page, {
      managedAgents: [
        {
          pubkey:
            "a110000000000000000000000000000000000000000000000000000000000042",
          name: "Pacioli Agent",
          status: "running",
          channelNames: ["agents"],
        },
      ],
    });
    await page.goto("/");
    await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  });

  test("stays mounted across close, resizes, refreshes, and restores after full screen", async ({
    page,
  }) => {
    const dock = page.getByTestId("business-dock");
    await expect(dock).toBeHidden();

    await page.getByTestId("business-dock-toggle").click();
    await expect(dock).toBeVisible();
    const frame = page.frameLocator('[data-testid="business-dock-iframe"]');
    await expect(
      frame.getByRole("heading", { name: "Business System Mock" }),
    ).toBeVisible();
    await expect(frame.locator("#bridge")).toHaveText("connected-v2");

    const initialWidth = (await dock.boundingBox())?.width ?? 0;
    const handle = page.getByTestId("business-dock-resize-handle");
    const handleBox = await handle.boundingBox();
    if (!handleBox)
      throw new Error("Business Dock resize handle is not laid out");
    const handleCenterX = handleBox.x + handleBox.width / 2;
    const handleCenterY = handleBox.y + handleBox.height / 2;
    await page.mouse.move(handleCenterX, handleCenterY);
    await page.mouse.down();
    const resizeIndicator = page.getByTestId("horizontal-resize-indicator");
    await expect(resizeIndicator).toBeVisible();
    await expect(resizeIndicator).toHaveText(/\d+ px · \d+%/);
    await page.mouse.move(handleCenterX - 120, handleCenterY, { steps: 10 });
    await expect(resizeIndicator).toHaveText(/\d+ px · \d+%/);
    await page.mouse.up();
    await expect(resizeIndicator).toBeHidden();
    await expect
      .poll(async () => (await dock.boundingBox())?.width ?? 0)
      .toBeGreaterThan(initialWidth);

    const expandedHandleBox = await handle.boundingBox();
    if (!expandedHandleBox)
      throw new Error("Business Dock resize handle disappeared");
    await page.mouse.move(
      expandedHandleBox.x + expandedHandleBox.width / 2,
      expandedHandleBox.y + expandedHandleBox.height / 2,
    );
    await page.mouse.down();
    await page.mouse.move(
      0,
      expandedHandleBox.y + expandedHandleBox.height / 2,
      {
        steps: 10,
      },
    );
    await page.mouse.up();
    await expect
      .poll(async () => Math.round((await dock.boundingBox())?.width ?? 0))
      .toBe(800);

    await handle.dblclick();
    await expect
      .poll(async () => Math.round((await dock.boundingBox())?.width ?? 0))
      .toBe(560);

    await page.getByLabel("Refresh business system").click();
    await expect(frame.locator("#refresh-count")).toHaveText("1");

    await frame
      .getByRole("button", { name: "Open sales order SO-1042" })
      .click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-1042",
    );

    await page.getByLabel("Full screen business system").click();
    await expect(dock).toHaveCSS("width", "1600px");
    await page.getByLabel("Exit full screen business system").click();
    await expect
      .poll(async () => Math.round((await dock.boundingBox())?.width ?? 0))
      .toBe(560);

    await page.getByLabel("Close Business Dock").click();
    await expect(dock).toBeHidden();
    await page.getByTestId("business-dock-toggle").click();
    await expect(frame.locator("#current-url")).toContainText(
      "/embed/sales-orders/SO-1042",
    );
  });

  test("opens message and Agent business links without changing the Buzz channel", async ({
    page,
  }) => {
    await emitBusinessMessage(
      page,
      "订单入口：[查看订单 SO-001](biz://sales-order/SO-001)",
    );
    await page.getByRole("link", { name: "查看订单 SO-001" }).click();
    await expect(page.getByTestId("business-dock")).toBeVisible();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-001",
    );
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    await emitBusinessMessage(
      page,
      "发现一笔异常订单：\n\n[查看 SO-002](biz://sales-order/SO-002)",
      "a110000000000000000000000000000000000000000000000000000000000042",
    );
    await page.getByRole("link", { name: "查看 SO-002" }).click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-002",
    );
    await expect(page.getByTestId("chat-title")).toHaveText("general");
  });

  test("resizes an open Thread and Business Dock independently", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 2200, height: 900 });
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const rootMessage = page
      .getByTestId("message-timeline")
      .getByTestId("message-row")
      .first();
    await rootMessage.hover();
    await rootMessage.getByRole("button", { name: "Reply" }).click();

    const thread = page.getByTestId("message-thread-panel");
    const threadHandle = page.getByTestId("right-auxiliary-pane-resize-handle");
    await expect(thread).toBeVisible();

    await page.getByTestId("business-dock-toggle").click();
    const dock = page.getByTestId("business-dock");
    const dockHandle = page.getByTestId("business-dock-resize-handle");
    await expect(dock).toBeVisible();

    const threadWidthBefore = (await thread.boundingBox())?.width ?? 0;
    const threadHandleBox = await threadHandle.boundingBox();
    if (!threadHandleBox)
      throw new Error("Thread resize handle is not laid out");
    const threadHandleY = threadHandleBox.y + threadHandleBox.height / 2;
    await page.mouse.move(threadHandleBox.x + 4, threadHandleY);
    await page.mouse.down();
    await page.mouse.move(threadHandleBox.x - 120, threadHandleY, {
      steps: 10,
    });
    await page.mouse.up();
    await expect
      .poll(async () => (await thread.boundingBox())?.width ?? 0)
      .toBeGreaterThan(threadWidthBefore + 100);

    const dockWidthBefore = (await dock.boundingBox())?.width ?? 0;
    const dockHandleBox = await dockHandle.boundingBox();
    if (!dockHandleBox)
      throw new Error("Business Dock resize handle is not laid out");
    const dockHandleY = dockHandleBox.y + dockHandleBox.height / 2;
    await page.mouse.move(dockHandleBox.x + 4, dockHandleY);
    await page.mouse.down();
    await page.mouse.move(dockHandleBox.x - 120, dockHandleY, { steps: 10 });
    await page.mouse.up();
    await expect
      .poll(async () => (await dock.boundingBox())?.width ?? 0)
      .toBeGreaterThan(dockWidthBefore + 100);
  });

  test("keeps host business history with back and forward navigation", async ({
    page,
  }) => {
    await emitBusinessMessage(
      page,
      "[SO-001](biz://sales-order/SO-001) · [SO-002](biz://sales-order/SO-002)",
    );
    await page.getByRole("link", { name: "SO-001", exact: true }).click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-001",
    );
    await page.getByRole("link", { name: "SO-002", exact: true }).click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-002",
    );
    await page.getByLabel("Business back").click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-001",
    );
    await page.getByLabel("Business forward").click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-002",
    );
  });

  test("accepts resource and action events from the V2 business bridge", async ({
    page,
  }) => {
    await page.getByTestId("business-dock-toggle").click();
    const frame = page.frameLocator('[data-testid="business-dock-iframe"]');
    await expect(frame.locator("#bridge")).toHaveText("connected-v2");
    await frame.getByRole("button", { name: "Change Resource" }).click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "客户 · CUST-2048",
    );
    await frame.getByRole("button", { name: "Action Success" }).click();
    await expect(page.getByText("经营异常已确认收到")).toBeVisible();
  });

  test("renders the V6 lifecycle, confirmation preview, work item, and draft-only approval", async ({
    page,
  }) => {
    await page.getByTestId("business-dock-toggle").click();
    const frame = page.frameLocator('[data-testid="business-dock-iframe"]');
    await expect(frame.getByText("Desensitized Acceptance UI")).toBeVisible();
    await expect(frame.getByText("Production Disabled")).toBeVisible();

    await frame.getByRole("button", { name: "Open anomaly finding" }).click();
    await expect(frame.locator("#lifecycle-kind")).toHaveText(
      "Finding lifecycle",
    );
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "经营异常 · FIND-001",
    );

    await frame.getByRole("button", { name: "Open action proposal" }).click();
    await expect(frame.locator("#lifecycle-kind")).toHaveText(
      "System suggestion",
    );
    await frame
      .getByRole("button", { name: "Prepare Work Item Preview" })
      .click();
    await expect(frame.locator("#lifecycle-title")).toHaveText("待办创建预览");

    await frame.getByRole("button", { name: "Confirm Work Item" }).click();
    await expect(frame.locator("#lifecycle-kind")).toHaveText(
      "Confirmed internal work item",
    );
    await expect(
      page.getByText("待办 WI-001 已由当前用户确认创建"),
    ).toBeVisible();

    await frame.getByRole("button", { name: "Open approval draft" }).click();
    await expect(frame.locator("#draft-only-warning")).toBeVisible();
    await expect(frame.locator("body")).not.toContainText("Approve");
    await expect(frame.locator("body")).not.toContainText("Execute");
  });

  test("handles Business Bridge V3 auth status without token handoff", async ({
    page,
  }) => {
    await page.getByTestId("business-dock-toggle").click();
    const frame = page.frameLocator('[data-testid="business-dock-iframe"]');
    await expect(frame.locator("#auth-status")).toHaveText("authenticated");
    await expect(page.getByTestId("business-auth-required")).toBeHidden();
    await frame.getByRole("button", { name: "Expire Session" }).click();
    await expect(page.getByTestId("business-auth-required")).toContainText(
      "Business session expired",
    );
    // The production recovery happens outside the iframe in a top-level/system
    // browser. Trigger the fixture control programmatically to model that
    // external callback without clicking through the host's auth overlay.
    await frame
      .getByRole("button", { name: "Authenticate" })
      .evaluate((button: HTMLButtonElement) => button.click());
    await expect(page.getByTestId("business-auth-required")).toBeHidden();
  });

  test("protects dirty business navigation on cancel and confirmation", async ({
    page,
  }) => {
    await emitBusinessMessage(
      page,
      "[Dirty SO-001](biz://sales-order/SO-001) · [Next SO-002](biz://sales-order/SO-002)",
    );
    await page.getByRole("link", { name: "Dirty SO-001" }).click();
    const frame = page.frameLocator('[data-testid="business-dock-iframe"]');
    await frame.getByRole("button", { name: "Mark Dirty" }).click();
    await expect(
      page.getByTestId("business-dock-dirty-indicator"),
    ).toBeVisible();

    await page.getByRole("link", { name: "Next SO-002" }).click();
    const dialog = page.getByTestId("business-dock-dirty-dialog");
    await expect(dialog).toBeVisible();
    await dialog.getByRole("button", { name: "取消" }).click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-001",
    );

    await page.getByRole("link", { name: "Next SO-002" }).click();
    await dialog.getByRole("button", { name: "仍然离开" }).click();
    await expect(page.getByTestId("business-resource-label")).toHaveText(
      "销售订单 · SO-002",
    );
  });

  test("coexists with a Buzz channel and thread surface", async ({ page }) => {
    await openGeneralThread(page);
    await page.getByTestId("business-dock-toggle").click();
    await expect(page.getByTestId("business-dock")).toBeVisible();

    const channelSurface = page.getByTestId("channel-drop-zone");
    const threadPanel = page.getByTestId("message-thread-panel");
    await expect(channelSurface).toBeVisible();
    await expect(threadPanel).toBeVisible();

    const channelMessage = `Channel while Dock open ${Date.now()}`;
    await channelSurface.getByTestId("message-input").fill(channelMessage);
    await channelSurface.getByTestId("send-message").click();
    await expect(channelSurface.getByTestId("message-timeline")).toContainText(
      channelMessage,
    );

    const threadReply = `Thread while Dock open ${Date.now()}`;
    await threadPanel.getByTestId("message-input").fill(threadReply);
    await threadPanel.getByTestId("send-message").click();
    await expect(threadPanel).toContainText(threadReply);
  });

  test("coexists with the Agents view and agent profile panel", async ({
    page,
  }) => {
    await page.getByTestId("open-agents-view").click();
    await expect(page.getByTestId("agents-page-content")).toBeVisible();
    const agentProfile = page
      .getByRole("button", { name: / agent profile$/ })
      .first();
    await expect(agentProfile).toBeVisible();
    await agentProfile.click();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible();

    await page.getByTestId("business-dock-toggle").click();
    await expect(page.getByTestId("business-dock")).toBeVisible();
    await expect(page.getByTestId("user-profile-panel")).toBeVisible();

    await page.getByTestId("user-profile-tab-runtime").click();
    await expect(
      page.getByTestId("user-profile-runtime-sections"),
    ).toBeVisible();
  });

  test("uses an overlay below 1000px and supports the global shortcut", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 900, height: 700 });
    await page.keyboard.press("Control+Shift+B");

    const dock = page.getByTestId("business-dock");
    await expect(dock).toBeVisible();
    await expect(dock).toHaveCSS("position", "absolute");
    await expect
      .poll(async () => Math.round((await dock.boundingBox())?.width ?? 0))
      .toBeLessThanOrEqual(900);

    await page
      .getByLabel("Close Business Dock overlay")
      .click({ position: { x: 24, y: 350 } });
    await expect(dock).toBeHidden();
  });
});
