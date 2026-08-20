import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.use({ video: "on" });

const CHANNEL_ID = "7b4d3d2e-3c15-4e88-9a11-1c0f7d2b6e44";
const ROOT_EVENT_ID = "a1".repeat(32);
const TARGET_EVENT_ID = "b2".repeat(32);
const MESSAGE_LINK = `buzz://message?channel=${CHANNEL_ID}&id=${TARGET_EVENT_ID}&thread=${ROOT_EVENT_ID}`;
const CANVAS = `# Slice 7 message-linked blocker demo

\`\`\`buzz-workstream-card
${JSON.stringify({
  version: 1,
  synopsis: "Open the exact blocker thread and reply in place.",
  orchestrator: { pubkey: "3".repeat(64), name: "Other Brother Darryl" },
  assignees: [{ pubkey: "4".repeat(64), name: "Logan" }],
  pullRequests: [],
  waitingOn: [
    {
      key: "loganj",
      actor: { kind: "person", name: "Logan", pubkey: "5".repeat(64) },
      reason: "Reply with the decision in the cited thread",
      since: "2026-08-19T21:00:00.000Z",
      message: MESSAGE_LINK,
    },
  ],
})}
\`\`\``;

async function openBoard(page: Page) {
  await installMockBridge(page, {
    workstreamBoardFixture: {
      channelId: CHANNEL_ID,
      channelName: "loganj-ws-s7-message-links",
      canvas: CANVAS,
      rootEventId: ROOT_EVENT_ID,
      targetEventId: TARGET_EVENT_ID,
    },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await page.getByTestId("open-workstream-board-view").click();
  await expect(page.getByTestId("workstream-board-view")).toBeVisible();
  const card = page.getByTestId(`workstream-card-${CHANNEL_ID}`);
  await expect(card).toBeVisible();
  return card.getByTestId("workstream-wait-loganj").locator("button");
}

async function expectTargetThreadAndComposer(page: Page) {
  await expect(page).toHaveURL(
    new RegExp(
      `/channels/${CHANNEL_ID}\\?messageId=${TARGET_EVENT_ID}&threadRootId=${ROOT_EVENT_ID}`,
    ),
  );
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await expect(page.getByText("Slice 7 exact reply target")).toBeVisible();
  const composer = page.getByTestId("thread-composer-overlay");
  await expect(composer).toBeVisible();
  await expect(composer.getByTestId("message-input")).toBeVisible();
  await expect(composer.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "true",
  );
}

test("message-linked blocker opens exact target thread by mouse", async ({
  page,
}) => {
  const blocker = await openBoard(page);
  await blocker.click();
  await expectTargetThreadAndComposer(page);
});

test("message-linked blocker opens exact target thread by keyboard", async ({
  page,
}) => {
  const blocker = await openBoard(page);
  await blocker.focus();
  await expect(blocker).toBeFocused();
  await blocker.press("Enter");
  await expectTargetThreadAndComposer(page);
});
