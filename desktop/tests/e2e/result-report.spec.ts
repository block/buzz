import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("presents the signed work result before the conversation", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);

  const rootId = await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock message emitter is unavailable");
    const root = emit({
      channelName: "general",
      content: "Implement result-first reporting",
      createdAt: 1_700_910_000,
    });
    emit({
      channelName: "general",
      content: "Implementation details and review discussion",
      parentEventId: root.id,
      createdAt: 1_700_910_001,
    });
    emit({
      channelName: "general",
      content: JSON.stringify({
        status: "completed",
        outcome: "Result reports now appear before long thread transcripts.",
        deliverables: ["https://example.com/pull/7057"],
        decisions: ["Keep the signed transcript as evidence."],
        verification: ["Desktop tests passed at abc1234."],
        risks: [],
        next_actions: ["Maintainer: review the pull request."],
      }),
      parentEventId: root.id,
      kind: 40009,
      extraTags: [
        ["e", root.id, "", "root"],
        ["t", "work-report"],
        ["status", "completed"],
      ],
      createdAt: 1_700_910_002,
    });
    return root.id;
  });

  await page.getByTestId("channel-general").click();
  const summary = page.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible();
  await summary.click();

  const card = page.getByTestId("work-report-card");
  await expect(card).toBeVisible();
  await expect(card).toContainText(
    "Result reports now appear before long thread transcripts.",
  );
  await expect(card).toContainText("Desktop tests passed at abc1234.");
  await expect(card).toContainText("Reported by");
  await expect(page.getByTestId("message-thread-head")).toBeHidden();

  await page.getByTestId("work-report-conversation-toggle").click();
  await expect(page.getByTestId("message-thread-head")).toBeVisible();
  await expect(
    page.getByText("Implementation details and review discussion"),
  ).toBeVisible();
});
