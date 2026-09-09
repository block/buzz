import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const TASK_CANVAS = `---
buzz_schema: channel-backed-task/v1
task:
  title: "Add status sounds to Berd Voice"
  description: "Move status-sound ownership into the Berd Voice runtime."
parent_channel: "buzz://channel/parent"
originating_thread: "buzz://message?channel=parent&id=origin"
branch:
  repository: "https://github.com/block/berd"
  name: "jtennant/berd-voice-status-sounds"
---`;

test("a task canvas exposes overview and conversation only", async ({
  page,
}) => {
  await installMockBridge(page, { canvasContent: TASK_CANVAS });
  await page.goto("/");
  await page.getByTestId("channel-engineering").click();

  await expect(page.getByTestId("task-overview")).toContainText(
    "Add status sounds to Berd Voice",
  );
  await expect(page.getByTestId("task-view-changes")).toHaveCount(0);
  await expect(page.getByTestId("task-view-review")).toHaveCount(0);
  await expect(page.getByTestId("task-overview")).toContainText(
    "jtennant/berd-voice-status-sounds",
  );
  await expect(page.getByTestId("task-work-tree")).toHaveCount(0);
  await page.getByTestId("task-view-conversation").click();
  await expect(page.getByTestId("message-timeline")).toBeVisible();
  await expect(page.getByTestId("task-work-navigator")).toHaveCount(0);
});

test("task canvases nest channels beneath their parents", async ({ page }) => {
  const parentId = "cf63feec-21bb-5bf0-a2f8-0e4c3de8ec73";
  const childId = "1c7e1c02-87bb-5e88-b2da-5a7a9432d0c9";
  const grandchildId = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
  const canvas = (parent: string) =>
    TASK_CANVAS.replace("buzz://channel/parent", `buzz://channel/${parent}`);

  await installMockBridge(page, {
    canvasContentByChannelId: {
      [childId]: canvas(parentId),
      [grandchildId]: canvas(childId),
    },
  });
  await page.goto("/");

  const list = page.getByTestId("stream-list");
  await expect(list.getByTestId("channel-buzz")).toBeVisible();
  await expect(
    list.getByTestId("channel-buzz").getByTestId("task-channel-status"),
  ).toHaveCount(0);
  await expect(
    list.getByTestId("channel-engineering").getByTestId("task-channel-status"),
  ).toBeVisible();
  await expect(
    list.locator(
      '[data-channel-depth="1"] [data-testid="channel-engineering"]',
    ),
  ).toBeVisible();
  await expect(
    list.locator('[data-channel-depth="2"] [data-testid="channel-agents"]'),
  ).toBeVisible();
  await list.getByTestId("channel-buzz").click();
  await expect(page.getByTestId("task-work-tree")).toHaveCount(0);
  await expect(page.getByTestId("task-view-changes")).toHaveCount(0);
  await expect(page.getByTestId("task-view-review")).toHaveCount(0);
});
