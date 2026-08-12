import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const RANDOM_CHANNEL_ID = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
const DEEP_HISTORY_CHANNEL_ID = "feedf00d-0000-4000-8000-000000000007";

type MessageRequest = {
  command: "get_channel_window" | "get_thread_replies";
  endedAt: number | null;
  payload: unknown;
  requestId: number;
  startedAt: number;
};

async function clearMessageRequests(page: Page) {
  await page.evaluate(() => {
    window.__BUZZ_E2E_MESSAGE_REQUEST_LOG__ = [];
  });
}

async function readMessageRequests(
  page: Page,
  command: MessageRequest["command"],
): Promise<MessageRequest[]> {
  return page.evaluate(
    (targetCommand) =>
      (window.__BUZZ_E2E_MESSAGE_REQUEST_LOG__ ?? []).filter(
        (entry) => entry.command === targetCommand,
      ),
    command,
  );
}

async function openThread(root: Locator, page: Page) {
  const timelineRoot = root.first();
  await timelineRoot.hover();
  await timelineRoot.getByRole("button", { name: "Reply" }).click();
  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  return panel;
}

async function installChannelRevisitPaintTracking(page: Page) {
  await page.evaluate(() => {
    const tracking = {
      cachedRowSeen: false,
      cachedRowSeenAt: null as number | null,
      cachedRowRemovedAfterPaint: false,
      skeletonSeen: false,
      disconnect: () => {},
    };
    const inspect = () => {
      const cachedRow = document.querySelector(
        '[data-message-id^="mock-deep-history-"]',
      );
      if (cachedRow && !tracking.cachedRowSeen) {
        tracking.cachedRowSeen = true;
        tracking.cachedRowSeenAt = performance.now();
      }
      if (tracking.cachedRowSeen && !cachedRow) {
        tracking.cachedRowRemovedAfterPaint = true;
      }
      if (document.querySelector('[data-testid="message-timeline-loading"]')) {
        tracking.skeletonSeen = true;
      }
    };
    const observer = new MutationObserver(inspect);
    observer.observe(document.body, { childList: true, subtree: true });
    tracking.disconnect = () => observer.disconnect();
    inspect();
    (
      window as typeof window & {
        __MESSAGE_REVISIT_PAINT__?: typeof tracking;
      }
    ).__MESSAGE_REVISIT_PAINT__ = tracking;
  });
}

async function readChannelRevisitPaintTracking(page: Page) {
  return page.evaluate(() => {
    const tracking = (
      window as typeof window & {
        __MESSAGE_REVISIT_PAINT__?: {
          cachedRowSeen: boolean;
          cachedRowSeenAt: number | null;
          cachedRowRemovedAfterPaint: boolean;
          disconnect: () => void;
          skeletonSeen: boolean;
        };
      }
    ).__MESSAGE_REVISIT_PAINT__;
    if (!tracking) throw new Error("revisit paint observer was not installed");
    tracking.disconnect();
    return tracking;
  });
}

test("cold channel load keeps its skeleton; fresh and stale revisits keep cached rows painted", async ({
  page,
}) => {
  await installMockBridge(page, { initialChannelWindowDelayMs: 700 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(
    page.locator('[data-message-id="mock-general-welcome"]'),
  ).toBeVisible();

  await clearMessageRequests(page);
  await page.getByTestId("channel-deep-history").click();
  await expect(page.getByTestId("chat-title")).toHaveText("deep-history");
  // Negative guard: a genuinely cold query still holds the timeline skeleton.
  await expect(page.getByTestId("message-timeline-loading")).toBeVisible();
  await expect
    .poll(() => readMessageRequests(page, "get_channel_window"))
    .toHaveLength(1);
  await expect(
    page.locator('[data-message-id^="mock-deep-history-"]').first(),
  ).toBeVisible();
  await expect(page.getByTestId("message-timeline-loading")).toHaveCount(0);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await clearMessageRequests(page);
  await installChannelRevisitPaintTracking(page);

  // A fresh warm revisit paints its settled cache immediately with no skeleton,
  // then remains painted across the required post-subscription reconciliation.
  await page.getByTestId("channel-deep-history").click();
  await expect(
    page.locator('[data-message-id^="mock-deep-history-"]').first(),
  ).toBeVisible();
  await expect
    .poll(() => readMessageRequests(page, "get_channel_window"))
    .toHaveLength(1);
  await expect
    .poll(async () =>
      (await readMessageRequests(page, "get_channel_window")).every(
        (request) => request.endedAt !== null,
      ),
    )
    .toBe(true);
  await expect(
    page.locator('[data-message-id^="mock-deep-history-"]').first(),
  ).toBeVisible();
  const freshRequests = await readMessageRequests(page, "get_channel_window");
  const freshRevisit = await readChannelRevisitPaintTracking(page);
  expect(freshRevisit.cachedRowSeen).toBe(true);
  expect(freshRevisit.cachedRowSeenAt).not.toBeNull();
  expect(freshRevisit.cachedRowSeenAt).toBeLessThan(
    Math.min(...freshRequests.map((request) => request.endedAt ?? Infinity)),
  );
  expect(freshRevisit.cachedRowRemovedAfterPaint).toBe(false);
  expect(freshRevisit.skeletonSeen).toBe(false);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  // Make the now-inactive message cache older than its five-minute staleTime
  // while retaining the authoritative channel-window proof.
  await page.evaluate((channelId) => {
    const queryClient = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
      getQueryData: (key: readonly unknown[]) => unknown;
      removeQueries: (filters: { queryKey: readonly unknown[] }) => void;
      setQueryData: (
        key: readonly unknown[],
        data: unknown,
        options?: { updatedAt?: number },
      ) => void;
    };
    const key = ["channel-messages", channelId] as const;
    const cachedMessages = queryClient.getQueryData(key);
    queryClient.removeQueries({ queryKey: key });
    queryClient.setQueryData(key, cachedMessages, {
      updatedAt: Date.now() - 10 * 60 * 1_000,
    });
  }, DEEP_HISTORY_CHANNEL_ID);
  await clearMessageRequests(page);
  await installChannelRevisitPaintTracking(page);
  const staleRevisitStartedAt = await page.evaluate(() => performance.now());

  // A >5-minute stale revisit also paints cache without a skeleton while one
  // delayed initial-window reconciliation runs behind it.
  await page.getByTestId("channel-deep-history").click();
  await expect(
    page.locator('[data-message-id^="mock-deep-history-"]').first(),
  ).toBeVisible();
  const staleRevisit = await readChannelRevisitPaintTracking(page);
  expect(staleRevisit.cachedRowSeen).toBe(true);
  expect(staleRevisit.cachedRowRemovedAfterPaint).toBe(false);
  expect(staleRevisit.skeletonSeen).toBe(false);
  await expect
    .poll(async () => {
      const requests = await readMessageRequests(page, "get_channel_window");
      return requests.some(
        (request) =>
          request.startedAt >= staleRevisitStartedAt &&
          request.endedAt === null,
      );
    })
    .toBe(true);
  const staleRevalidation = (
    await readMessageRequests(page, "get_channel_window")
  ).find(
    (request) =>
      request.startedAt >= staleRevisitStartedAt && request.endedAt === null,
  );
  expect(staleRevalidation).toBeDefined();
  expect(staleRevalidation?.endedAt).toBeNull();
  await expect
    .poll(
      async () =>
        (await readMessageRequests(page, "get_channel_window")).find(
          (request) => request.requestId === staleRevalidation?.requestId,
        )?.endedAt,
    )
    .not.toBeNull();
});

test("thread cache paints on fresh reopen and while a stale cache revalidates", async ({
  page,
}) => {
  await installMockBridge(page, { threadRepliesDelayMs: 700 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const root = page.locator('[data-message-id="mock-general-welcome"]');
  await expect(root).toBeVisible();
  const reply = await page.evaluate((parentEventId) => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("mock message emitter is unavailable");
    return emit({
      channelName: "general",
      content: "Cached thread reply",
      parentEventId,
    });
  }, "mock-general-welcome");

  let panel = await openThread(root, page);
  await expect(panel.locator(`[data-message-id="${reply.id}"]`)).toContainText(
    "Cached thread reply",
  );
  await expect
    .poll(async () => {
      const requests = await readMessageRequests(page, "get_thread_replies");
      return requests.length === 1 && requests[0].endedAt !== null;
    })
    .toBe(true);
  await panel.getByTestId("auxiliary-panel-close").click();
  await expect(panel).toHaveCount(0);

  // Every reopen paints the cache immediately and revalidates in background.
  await clearMessageRequests(page);
  panel = await openThread(root, page);
  await expect(panel.locator(`[data-message-id="${reply.id}"]`)).toBeVisible();
  await expect(panel.getByTestId("message-thread-replies-loading")).toHaveCount(
    0,
  );
  await expect
    .poll(() => readMessageRequests(page, "get_thread_replies"))
    .toHaveLength(1);
  const [freshRevalidation] = await readMessageRequests(
    page,
    "get_thread_replies",
  );
  expect(freshRevalidation.endedAt).toBeNull();
  await expect
    .poll(
      async () =>
        (await readMessageRequests(page, "get_thread_replies"))[0]?.endedAt,
    )
    .not.toBeNull();

  // Negative guard: once genuinely stale, the same cached row remains painted
  // while a delayed background reconciliation is in flight.
  await panel.getByTestId("auxiliary-panel-close").click();
  await expect(panel).toHaveCount(0);
  await page.evaluate(
    async ({ channelId, rootId }) => {
      const queryClient = window.__BUZZ_E2E_QUERY_CLIENT__ as unknown as {
        getQueryData: (key: readonly unknown[]) => unknown;
        removeQueries: (filters: { queryKey: readonly unknown[] }) => void;
        setQueryData: (
          key: readonly unknown[],
          data: unknown,
          options?: { updatedAt?: number },
        ) => void;
      };
      const key = ["thread-replies", channelId, rootId] as const;
      const cachedReplies = queryClient.getQueryData(key);
      queryClient.removeQueries({ queryKey: key });
      queryClient.setQueryData(key, cachedReplies, {
        updatedAt: Date.now() - 10 * 60 * 1_000,
      });
    },
    { channelId: GENERAL_CHANNEL_ID, rootId: "mock-general-welcome" },
  );
  await clearMessageRequests(page);
  await page.evaluate((replyId) => {
    const tracking = {
      cachedReplyFrameAt: null as number | null,
      disconnect: () => {},
    };
    let framePending = false;
    const inspect = () => {
      if (tracking.cachedReplyFrameAt !== null || framePending) return;
      if (!document.querySelector(`[data-message-id="${replyId}"]`)) return;
      framePending = true;
      requestAnimationFrame(() => {
        tracking.cachedReplyFrameAt = performance.now();
        framePending = false;
      });
    };
    const observer = new MutationObserver(inspect);
    observer.observe(document.body, { childList: true, subtree: true });
    tracking.disconnect = () => observer.disconnect();
    inspect();
    (
      window as typeof window & {
        __THREAD_CACHE_PAINT__?: typeof tracking;
      }
    ).__THREAD_CACHE_PAINT__ = tracking;
  }, reply.id);

  panel = await openThread(root, page);
  await expect(panel.locator(`[data-message-id="${reply.id}"]`)).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (
            window as typeof window & {
              __THREAD_CACHE_PAINT__?: { cachedReplyFrameAt: number | null };
            }
          ).__THREAD_CACHE_PAINT__?.cachedReplyFrameAt ?? null,
      ),
    )
    .not.toBeNull();
  const cachedPaintedAt = await page.evaluate(() => {
    const tracking = (
      window as typeof window & {
        __THREAD_CACHE_PAINT__?: {
          cachedReplyFrameAt: number | null;
          disconnect: () => void;
        };
      }
    ).__THREAD_CACHE_PAINT__;
    if (!tracking?.cachedReplyFrameAt) {
      throw new Error("cached thread reply paint was not observed");
    }
    tracking.disconnect();
    return tracking.cachedReplyFrameAt;
  });
  await expect(panel.getByTestId("message-thread-replies-loading")).toHaveCount(
    0,
  );
  await expect
    .poll(() => readMessageRequests(page, "get_thread_replies"))
    .toHaveLength(1);
  await expect
    .poll(
      async () =>
        (await readMessageRequests(page, "get_thread_replies"))[0]?.endedAt,
    )
    .not.toBeNull();
  const [revalidation] = await readMessageRequests(page, "get_thread_replies");
  expect(revalidation.startedAt).toBeLessThanOrEqual(cachedPaintedAt);
  expect(revalidation.endedAt ?? 0).toBeGreaterThanOrEqual(cachedPaintedAt);
  await expect(panel.locator(`[data-message-id="${reply.id}"]`)).toBeVisible();
});

test("401 thread replies fit in one 500-row request", async ({ page }) => {
  await installMockBridge(page, { threadRepliesDelayMs: 25 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const seeded = await page.evaluate(() => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("mock message emitter is unavailable");
    const root = emit({ channelName: "random", content: "Large thread root" });
    for (let index = 0; index < 401; index += 1) {
      emit({
        channelName: "random",
        content: `Large thread reply ${index}`,
        createdAt: 1_700_000_000 + index,
        parentEventId: root.id,
      });
    }
    return { rootId: root.id };
  });

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  const root = page.locator(`[data-message-id="${seeded.rootId}"]`);
  await expect(root).toBeVisible();
  await clearMessageRequests(page);
  const panel = await openThread(root, page);
  await expect(panel).toContainText("Large thread reply 400");

  await expect
    .poll(() => readMessageRequests(page, "get_thread_replies"))
    .toHaveLength(1);
  const requests = await readMessageRequests(page, "get_thread_replies");
  expect(requests[0].endedAt).not.toBeNull();

  expect((requests[0].payload as { limit?: number }).limit).toBe(500);
  expect((requests[0].payload as { channelId?: string }).channelId).toBe(
    RANDOM_CHANNEL_ID,
  );
});
