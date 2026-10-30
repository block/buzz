import { expect, type Page, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
import { openGeneral, pressRowAction } from "../helpers/channelHeadReady";

const ROOT_ID = "mock-general-welcome";

// A nested-era branch: root <- child <- grandchild.
async function seedNestedThread(page: Page) {
  return page.evaluate(
    ({ rootId, alice, bob }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock message emitter is not installed");
      const past = Math.floor(Date.now() / 1000) - 60;
      const child = emit({
        channelName: "general",
        content: "Flat child",
        createdAt: past,
        parentEventId: rootId,
        pubkey: alice,
      });
      const grandchild = emit({
        channelName: "general",
        content: "Flat grandchild",
        createdAt: past + 1,
        parentEventId: child.id,
        pubkey: bob,
      });
      return { childId: child.id, grandchildId: grandchild.id };
    },
    {
      alice: TEST_IDENTITIES.alice.pubkey,
      bob: TEST_IDENTITIES.bob.pubkey,
      rootId: ROOT_ID,
    },
  );
}

function sentThreadParentIds(page: Page) {
  return page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])
      .filter(({ command }) => command === "send_channel_message")
      .map(
        ({ payload }) =>
          (payload as { parentEventId?: string | null }).parentEventId,
      ),
  );
}

test("flat communities show one reply level and reply to the thread root", async ({
  page,
}) => {
  await installMockBridge(page, { threadPolicy: "flat" });
  await page.goto("/");
  await openGeneral(page);
  const { childId, grandchildId } = await seedNestedThread(page);

  await pressRowAction(page.getByTestId(`reply-message-${ROOT_ID}`));
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  // Both replies are listed directly under the root — no collapsed branch.
  const replies = threadPanel.getByTestId("message-thread-replies");
  await expect(
    replies.locator(`[data-message-id="${grandchildId}"]`),
  ).toBeVisible();
  await expect(
    replies.locator(`[data-thread-head-id="${childId}"]`),
  ).toHaveCount(0);

  // The action bar stays mounted (opacity-hidden) for hover and focus, so an
  // absent button means no input modality can pick a reply as parent.
  await replies.locator(`[data-message-id="${childId}"]`).hover();
  await expect(threadPanel.getByTestId(`reply-message-${childId}`)).toHaveCount(
    0,
  );
  await expect(threadPanel.getByTestId(`reply-message-${ROOT_ID}`)).toHaveCount(
    0,
  );

  const composer = threadPanel.getByTestId("message-input");
  await composer.click();
  await page.keyboard.type("flat reply");
  await page.keyboard.press("Enter");
  await expect(threadPanel.getByText("flat reply")).toBeVisible();
  await expect(threadPanel.getByTestId("reply-target")).toHaveCount(0);
  expect(await sentThreadParentIds(page)).toEqual([ROOT_ID]);
});

test("nested communities keep the per-reply Reply action", async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
  await openGeneral(page);
  const { childId } = await seedNestedThread(page);

  await pressRowAction(page.getByTestId(`reply-message-${ROOT_ID}`));
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  await pressRowAction(threadPanel.getByTestId(`reply-message-${childId}`));
  await expect(threadPanel.getByTestId("reply-target")).toBeVisible();
});
