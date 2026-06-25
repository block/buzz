import { expect, test } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";

const SHOTS = "test-results/thread-reply-anchor-roleplay";
const SELF_PUBKEY = "deadbeef".repeat(8);
const CHANNEL = "general";

type MockMessageEvent = {
  id: string;
  created_at: number;
  pubkey: string;
};

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      );
    })
    .toBe(true);
}

async function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  options?: {
    parentEventId?: string | null;
    pubkey?: string;
    createdAt?: number;
    mentionPubkeys?: string[];
  },
): Promise<MockMessageEvent> {
  const event = await page.evaluate(
    ({ ch, msg, parentEventId, pubkey, ts, mentionPubkeys }) => {
      return (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            parentEventId?: string | null;
            pubkey?: string;
            createdAt?: number;
            mentionPubkeys?: string[];
          }) => { id: string; created_at: number; pubkey: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        parentEventId,
        pubkey,
        createdAt: ts,
        mentionPubkeys,
      });
    },
    {
      ch: channelName,
      msg: content,
      parentEventId: options?.parentEventId ?? null,
      pubkey: options?.pubkey ?? SELF_PUBKEY,
      ts: options?.createdAt,
      mentionPubkeys: options?.mentionPubkeys,
    },
  );
  if (!event) {
    throw new Error("Mock message emitter is not installed");
  }
  return event;
}

async function setupRoleplayChannel(page: import("@playwright/test").Page) {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "Pinky",
        respondTo: "anyone",
        channelNames: [CHANNEL],
        status: "online",
      },
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "Brain",
        respondTo: "anyone",
        channelNames: [CHANNEL],
        status: "online",
      },
    ],
    searchProfiles: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        displayName: "Pinky",
        isAgent: true,
      },
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        displayName: "Brain",
        isAgent: true,
      },
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        displayName: "Wes",
        isAgent: false,
      },
      {
        pubkey: SELF_PUBKEY,
        displayName: "Nora",
        isAgent: false,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL);
  await waitForMockLiveSubscription(page, CHANNEL);
}

async function openThread(page: import("@playwright/test").Page) {
  const summary = page.getByTestId("message-thread-summary").first();
  await expect(summary).toBeVisible();
  await summary.click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
}

async function expandReply(
  page: import("@playwright/test").Page,
  replyId: string,
) {
  const replies = page
    .getByTestId("message-thread-replies")
    .getByTestId("message-row");
  const before = await replies.count();
  await page.locator(`[data-thread-head-id="${replyId}"]`).click();
  await expect.poll(() => replies.count()).toBeGreaterThan(before);
}

async function screenshotThreadPanel(
  page: import("@playwright/test").Page,
  path: string,
) {
  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await page.mouse.move(360, 24);
  await page.waitForTimeout(100);
  await panel.screenshot({ path });
}

test.describe("thread reply anchor A/B roleplay screenshots", () => {
  test("01-baseline-human-reply-nests-agent-at-depth-2", async ({ page }) => {
    await setupRoleplayChannel(page);

    const now = Math.floor(Date.now() / 1000);
    const root = await emitMockMessage(
      page,
      CHANNEL,
      "Wes: @Pinky please review the checkout copy.",
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.alice.pubkey],
        createdAt: now,
      },
    );
    const humanReply = await emitMockMessage(
      page,
      CHANNEL,
      "Nora: adding context — this is only about the receipt screen.",
      {
        parentEventId: root.id,
        pubkey: SELF_PUBKEY,
        mentionPubkeys: [TEST_IDENTITIES.alice.pubkey],
        createdAt: now + 1,
      },
    );

    // Baseline queue.rs anchored the agent response to the triggering human
    // reply, producing depth 2 under Nora's message.
    await emitMockMessage(
      page,
      CHANNEL,
      "Pinky: Got it — I’ll check the receipt copy only. Narf!",
      {
        parentEventId: humanReply.id,
        pubkey: TEST_IDENTITIES.alice.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.bob.pubkey, SELF_PUBKEY],
        createdAt: now + 2,
      },
    );

    await openThread(page);
    await expandReply(page, humanReply.id);
    await expect(page.getByText("Nora: adding context")).toBeVisible();
    await expect(page.getByText("Pinky: Got it")).toBeVisible();
    await expect(
      page.getByTestId("message-thread-replies").getByTestId("message-row"),
    ).toHaveCount(2);
    await expect(page.getByTestId("thread-collapse-rail")).toHaveCount(2);

    await screenshotThreadPanel(page, `${SHOTS}/01-baseline-depth-2.png`);
  });

  test("02-patched-human-reply-flattens-agent-at-root", async ({ page }) => {
    await setupRoleplayChannel(page);

    const now = Math.floor(Date.now() / 1000);
    const root = await emitMockMessage(
      page,
      CHANNEL,
      "Wes: @Pinky please review the checkout copy.",
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.alice.pubkey],
        createdAt: now,
      },
    );
    await emitMockMessage(
      page,
      CHANNEL,
      "Nora: adding context — this is only about the receipt screen.",
      {
        parentEventId: root.id,
        pubkey: SELF_PUBKEY,
        mentionPubkeys: [TEST_IDENTITIES.alice.pubkey],
        createdAt: now + 1,
      },
    );

    // Patched queue.rs anchors the agent response to the thread root, keeping
    // both human and agent replies as flat layer-1 siblings.
    await emitMockMessage(
      page,
      CHANNEL,
      "Pinky: Got it — I’ll check the receipt copy only. Narf!",
      {
        parentEventId: root.id,
        pubkey: TEST_IDENTITIES.alice.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.bob.pubkey, SELF_PUBKEY],
        createdAt: now + 2,
      },
    );

    await openThread(page);
    await expect(page.getByText("Nora: adding context")).toBeVisible();
    await expect(page.getByText("Pinky: Got it")).toBeVisible();
    await expect(
      page.getByTestId("message-thread-replies").getByTestId("message-row"),
    ).toHaveCount(2);
    await expect(page.getByTestId("thread-collapse-rail")).toHaveCount(0);

    await screenshotThreadPanel(page, `${SHOTS}/02-patched-flat-l1.png`);
  });

  test("03-patched-top-level-human-starts-thread-at-human-root", async ({
    page,
  }) => {
    await setupRoleplayChannel(page);

    const now = Math.floor(Date.now() / 1000);
    const humanRoot = await emitMockMessage(
      page,
      CHANNEL,
      "Wes: @Pinky start the inventory audit.",
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.alice.pubkey],
        createdAt: now,
      },
    );
    await emitMockMessage(
      page,
      CHANNEL,
      "Pinky: Starting the audit and I’ll report back here. Poit!",
      {
        parentEventId: humanRoot.id,
        pubkey: TEST_IDENTITIES.alice.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.bob.pubkey],
        createdAt: now + 1,
      },
    );

    await openThread(page);
    await expect(page.getByText("Pinky: Starting the audit")).toBeVisible();
    await expect(
      page.getByTestId("message-thread-replies").getByTestId("message-row"),
    ).toHaveCount(1);

    await screenshotThreadPanel(page, `${SHOTS}/03-top-level-human-root.png`);
  });

  test("04-agent-only-branch-keeps-deeper-nesting", async ({ page }) => {
    await setupRoleplayChannel(page);

    const now = Math.floor(Date.now() / 1000);
    const root = await emitMockMessage(
      page,
      CHANNEL,
      "Pinky: @Brain I found a failing visual case.",
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.charlie.pubkey],
        createdAt: now,
      },
    );
    const brainReply = await emitMockMessage(
      page,
      CHANNEL,
      "Brain: Check the anchor emitted by queue.rs.",
      {
        parentEventId: root.id,
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.alice.pubkey],
        createdAt: now + 1,
      },
    );
    await emitMockMessage(
      page,
      CHANNEL,
      "Pinky: Good catch — agent-only branches can stay nested. Zort!",
      {
        parentEventId: brainReply.id,
        pubkey: TEST_IDENTITIES.alice.pubkey,
        mentionPubkeys: [TEST_IDENTITIES.charlie.pubkey],
        createdAt: now + 2,
      },
    );

    await openThread(page);
    await expandReply(page, brainReply.id);
    await expect(page.getByText("Brain: Check the anchor")).toBeVisible();
    await expect(page.getByText("Pinky: Good catch")).toBeVisible();
    await expect(
      page.getByTestId("message-thread-replies").getByTestId("message-row"),
    ).toHaveCount(2);
    await expect(page.getByTestId("thread-collapse-rail")).toHaveCount(2);

    await screenshotThreadPanel(page, `${SHOTS}/04-agent-only-nested.png`);
  });
});
