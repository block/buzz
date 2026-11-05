import { expect, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

type MockMessageWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    parentEventId?: string | null;
    createdAt?: number;
    pubkey?: string;
  }) => { id: string } | undefined;
  __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
    channelName: string;
    kind?: number;
  }) => boolean;
  __BUZZ_E2E_SET_REMOTE_THREAD_TITLE__?: (input: {
    channelName: string;
    rootId: string;
    title: string;
  }) => void;
};

const CHANNEL_NAME = "engineering";
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

async function openThread(
  page: Page,
  rootContent: string,
  options: {
    channelName?: string;
    replyCreatedAt?: number;
    replyPubkey?: string;
  } = {},
): Promise<string> {
  const channelName = options.channelName ?? CHANNEL_NAME;
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          (
            window as MockMessageWindow
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: name }) ??
          false,
        channelName,
      ),
    )
    .toBe(true);

  const summariesBefore = await page
    .getByTestId("message-thread-summary")
    .count();
  const rootId = await page.evaluate(
    ({ channelName, content, pubkey }) =>
      (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content,
        pubkey,
      })?.id ?? null,
    {
      channelName,
      content: rootContent,
      pubkey: MOCK_IDENTITY_PUBKEY,
    },
  );
  expect(rootId).not.toBeNull();
  await page.evaluate(
    ({ channelName, createdAt, parentEventId, pubkey }) => {
      (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName,
        content: "First reply.",
        createdAt,
        parentEventId,
        pubkey,
      });
    },
    {
      channelName,
      createdAt: options.replyCreatedAt,
      parentEventId: rootId,
      pubkey: options.replyPubkey ?? ALICE_PUBKEY,
    },
  );
  // A new root collapses its reply into a summary row as it renders, so a
  // click on the inline reply can land on a detached node. Wait for the
  // summary instead; `.last()` alone could still resolve to an earlier one.
  const summaries = page.getByTestId("message-thread-summary");
  await expect(summaries).toHaveCount(summariesBefore + 1);
  await summaries.last().click();
  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText(rootContent);
  return rootId as string;
}

test.describe("thread titles", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("name a thread from its header, then clear it", async ({ page }) => {
    await installMockBridge(page);
    await page.goto("/");
    await openThread(page, "Root message that will get a title.");

    const panel = page.getByTestId("message-thread-panel");
    await expect(panel.getByTestId("thread-title")).toHaveText("Thread");

    await panel.getByTestId("thread-title").hover();
    await panel.getByTestId("thread-title-edit").click();
    const input = panel.getByTestId("thread-title-input");
    await expect(input).toBeFocused();
    await input.fill("  Release checklist  ");
    await input.press("Enter");
    await expect(panel.getByTestId("thread-title")).toHaveText(
      "Release checklist",
    );
    // The root message in the channel carries the title on its own line, and
    // the thread panel's copy of the root doesn't repeat it under the header.
    const titleLine = page.getByTestId("thread-title-line");
    await expect(titleLine).toHaveCount(1);
    await expect(titleLine).toHaveText("Release checklist");
    await expect(panel.getByTestId("thread-title-line")).toHaveCount(0);

    // Escape abandons an edit without writing.
    await panel.getByTestId("thread-title").hover();
    await panel.getByTestId("thread-title-edit").click();
    await panel.getByTestId("thread-title-input").fill("Discarded");
    await panel.getByTestId("thread-title-input").press("Escape");
    await expect(panel.getByTestId("thread-title")).toHaveText(
      "Release checklist",
    );

    // A blank title clears it everywhere.
    await panel.getByTestId("thread-title").hover();
    await panel.getByTestId("thread-title-edit").click();
    await panel.getByTestId("thread-title-input").fill("   ");
    await panel.getByTestId("thread-title-input").press("Enter");
    await expect(panel.getByTestId("thread-title")).toHaveText("Thread");
    await expect(titleLine).toHaveCount(0);
  });

  test("a title another user sets appears before anyone replies", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL_NAME}`).click();
    await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL_NAME);
    await expect
      .poll(() =>
        page.evaluate(
          (name) =>
            (
              window as MockMessageWindow
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: name,
              kind: 45010,
            }) ?? false,
          CHANNEL_NAME,
        ),
      )
      .toBe(true);

    const rootId = await page.evaluate(
      ({ channelName, pubkey }) =>
        (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content: "Question for an agent, no reply yet.",
          pubkey,
        })?.id ?? null,
      { channelName: CHANNEL_NAME, pubkey: MOCK_IDENTITY_PUBKEY },
    );
    expect(rootId).not.toBeNull();
    await expect(
      page.getByText("Question for an agent, no reply yet."),
    ).toBeVisible();

    await page.evaluate(
      ({ channelName, rootId }) =>
        (window as MockMessageWindow).__BUZZ_E2E_SET_REMOTE_THREAD_TITLE__?.({
          channelName,
          rootId,
          title: "Agent-named thread",
        }),
      { channelName: CHANNEL_NAME, rootId: rootId as string },
    );

    // The untitled root's lookup is cached for 30s, so only the live
    // refresh can deliver the title this quickly.
    await expect(page.getByTestId("thread-title-line")).toHaveText(
      "Agent-named thread",
      { timeout: 5_000 },
    );
  });
});
