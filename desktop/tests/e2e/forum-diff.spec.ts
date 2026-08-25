import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const WATERCOOLER_CHANNEL_ID = "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11";
const FORUM_POST_ID = "mock-forum-release-thread";

const DIFF_CONTENT = `diff --git a/src/hive.ts b/src/hive.ts
index 1111111..2222222 100644
--- a/src/hive.ts
+++ b/src/hive.ts
@@ -1,3 +1,3 @@
 export function hive() {
-  return "old";
+  return "new";
 }
`;

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

/**
 * Seeds a kind-40008 diff reply on the seeded forum thread and opens the
 * thread scrolled to it. The thread carries 25 seeded replies and rows use
 * `content-visibility: auto`, so the reply must be deep-linked by id rather
 * than searched for at the bottom of the list.
 */
async function openThreadAtDiffReply(
  page: import("@playwright/test").Page,
  extraTags: string[][],
) {
  await page.goto("/");

  // The bridge installs its emit hook after navigation settles; without this
  // wait the evaluate below can run against a window that has no hook yet.
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  const replyId = await page.evaluate(
    ({ parentEventId, content, tags }) => {
      const event = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "watercooler",
        content,
        kind: 40008,
        parentEventId,
        extraTags: tags,
      });
      return event?.id ?? null;
    },
    { parentEventId: FORUM_POST_ID, content: DIFF_CONTENT, tags: extraTags },
  );

  expect(replyId).toBeTruthy();

  await page.goto(
    `/#/channels/${WATERCOOLER_CHANNEL_ID}/posts/${FORUM_POST_ID}?replyId=${replyId}`,
  );

  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
}

// Diffs (kind 40008) used to be fetched only by the stream timeline, so a diff
// posted into a forum thread was stored but invisible. The thread reply filter
// now includes 40008 and ReplyRow renders it through DiffMessage.
test("forum threads render a diff reply as a diff card", async ({ page }) => {
  await openThreadAtDiffReply(page, [
    ["repo", "https://github.com/block/buzz"],
    ["commit", "abcdef1234567890abcdef1234567890abcdef12"],
    ["file", "src/hive.ts"],
    ["description", "Return the new value"],
  ]);

  // The diff card chrome — file path, short SHA, description, expand control —
  // only exists on the DiffMessage path. Markdown rendering would show the raw
  // `diff --git` text instead.
  await expect(page.getByText("src/hive.ts").first()).toBeVisible();
  await expect(page.getByText("abcdef1").first()).toBeVisible();
  await expect(page.getByText("Return the new value")).toBeVisible();
  await expect(page.getByRole("button", { name: "Expand diff" })).toBeVisible();
});

test("expanding a forum diff reply opens the full diff viewer", async ({
  page,
}) => {
  await openThreadAtDiffReply(page, [["file", "src/hive.ts"]]);

  await page.getByRole("button", { name: "Expand diff" }).click();

  await expect(page.getByRole("dialog")).toBeVisible();
});
