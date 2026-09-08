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

test("a task canvas turns one channel into task, changes, review, and conversation views", async ({
  page,
}) => {
  await installMockBridge(page, { canvasContent: TASK_CANVAS });
  await page.goto("/");
  await page.getByTestId("channel-engineering").click();

  await expect(page.getByTestId("task-overview")).toContainText(
    "Add status sounds to Berd Voice",
  );
  await page.getByTestId("task-view-changes").click();
  await expect(page.getByTestId("task-changes")).toContainText(
    "ProjectDetailScreen.tsx",
  );
  await page.getByTestId("task-view-review").click();
  await expect(page.getByText("No linked review")).toBeVisible();
  await page.getByTestId("task-view-conversation").click();
  await expect(page.getByTestId("message-timeline")).toBeVisible();
});
