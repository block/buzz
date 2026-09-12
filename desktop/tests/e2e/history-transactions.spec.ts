import type { QueryClient } from "@tanstack/react-query";
import { expect, test, type Page } from "@playwright/test";
import type { ChannelWindowStore } from "../../src/features/messages/lib/channelWindowStore";
import type { RelayEvent } from "../../src/shared/api/types";
import { installMockBridge } from "../helpers/bridge";
import {
  pageOlderHistory,
  startOlderHistory,
  waitForHistorySettled,
} from "../helpers/timelineHistory";

async function visibleAnchor(page: Page) {
  return page.getByTestId("message-timeline").evaluate((element) => {
    const top = element.getBoundingClientRect().top;
    const row = [
      ...element.querySelectorAll<HTMLElement>("[data-message-id]"),
    ].find((row) => row.getBoundingClientRect().bottom > top + 60);
    if (!row?.dataset.messageId) throw Error("No visible anchor");
    return {
      id: row.dataset.messageId,
      top: row.getBoundingClientRect().top - top,
    };
  });
}

async function anchorTop(page: Page, id: string) {
  return page.getByTestId("message-timeline").evaluate((element, id) => {
    const row = element.querySelector(`[data-message-id="${CSS.escape(id)}"]`);
    return row
      ? row.getBoundingClientRect().top - element.getBoundingClientRect().top
      : null;
  }, id);
}

async function beginAnchorTrace(page: Page, id: string) {
  await page.evaluate((id) => {
    const trace: Array<number | null> = [];
    Object.assign(window, { __HISTORY_ANCHOR_TRACE__: trace });
    const sample = () => {
      const scroller = document.querySelector(
        '[data-testid="message-timeline"]',
      );
      const row = scroller?.querySelector(
        `[data-message-id="${CSS.escape(id)}"]`,
      );
      trace.push(
        row && scroller
          ? row.getBoundingClientRect().top -
              scroller.getBoundingClientRect().top
          : null,
      );
      if (trace.length < 600) requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  }, id);
}

async function assertAnchorTrace(page: Page, top: number) {
  const trace = await page.evaluate(
    () =>
      (window as unknown as { __HISTORY_ANCHOR_TRACE__: Array<number | null> })
        .__HISTORY_ANCHOR_TRACE__,
  );
  expect(trace.length).toBeGreaterThan(3);
  expect(
    trace.every((value) => value !== null),
    JSON.stringify(trace),
  ).toBe(true);
  expect(
    Math.max(...trace.map((value) => Math.abs((value ?? Infinity) - top))),
    JSON.stringify(trace),
  ).toBeLessThan(5);
}

for (const race of [false, true]) {
  test(`reconnect preserves deep history${race ? " with an older request in flight" : ""}`, async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-deep-history").click();
    await expect(
      page.getByTestId("message-timeline").locator("[data-message-id]").first(),
    ).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "deep-history",
            }) ?? false,
        ),
      )
      .toBe(true);
    for (let step = 0; step < 4; step++) await pageOlderHistory(page);
    await page.evaluate(() => {
      window.__BUZZ_E2E__ = {
        ...window.__BUZZ_E2E__,
        mock: { ...window.__BUZZ_E2E__?.mock, channelWindowDelayMs: 800 },
      };
    });
    if (race) {
      await startOlderHistory(page);
      // Wheel delivery is asynchronous; sample the resting anchor only after
      // the input reached the boundary, still before the delayed response.
      await expect
        .poll(() =>
          page
            .getByTestId("message-timeline")
            .evaluate((element) => element.scrollTop),
        )
        .toBe(0);
    }
    const anchor = await visibleAnchor(page);
    await beginAnchorTrace(page, anchor.id);
    const headsBefore = await page.evaluate(
      () =>
        (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
          (entry) =>
            entry.command === "get_channel_window" &&
            !(entry.payload as { cursor?: unknown })?.cursor,
        ).length,
    );
    await page.evaluate(() => {
      // Move page zero's cursor, forcing a fresh bridging page instead of
      // attaching the old cursor chain to an unrelated head.
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "deep-history",
        content: "head moved while reading",
        createdAt: Math.floor(Date.now() / 1000) + 1,
      });
      window.__BUZZ_E2E_RESTART_MOCK_WEBSOCKETS__?.();
    });
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
              (entry) =>
                entry.command === "get_channel_window" &&
                !(entry.payload as { cursor?: unknown })?.cursor,
            ).length,
        ),
      )
      .toBeGreaterThan(headsBefore);
    await waitForHistorySettled(page);
    // Head revalidation may need several fresh cursor windows. Its staged
    // publication must never shrink the reader to the newest 50 rows.
    await page.waitForTimeout(4500);
    expect(await anchorTop(page, anchor.id)).not.toBeNull();
    await assertAnchorTrace(page, anchor.top);
    // A retired request/gesture cannot poison subsequent intentional paging.
    await pageOlderHistory(page);
  });
}

test("reconnect inserts a middle history row above a deep reader without shifting", async ({
  page,
}, testInfo) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-deep-history").click();
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  for (let step = 0; step < 4; step++) await pageOlderHistory(page);

  // Retain five pages, then read inside the first revalidated join page. Deeper
  // exact joins deliberately reuse immutable history; this is a reconnect gap
  // in the range actually fetched, not a request for arbitrary backfill repair.
  await timeline.evaluate((element) => {
    element.scrollTop = element.scrollHeight * 0.66;
  });
  const readingRow = timeline.locator(
    '[data-message-id="mock-deep-history-520"]',
  );
  await expect(readingRow).toBeAttached();
  await readingRow.evaluate((row) => {
    const scroller = row.closest('[data-testid="message-timeline"]');
    if (!scroller) throw Error("No timeline scroller");
    scroller.scrollTop +=
      row.getBoundingClientRect().top -
      scroller.getBoundingClientRect().top -
      100;
  });
  await waitForHistorySettled(page);
  const anchor = await visibleAnchor(page);
  const channelId = "feedf00d-0000-4000-8000-000000000007";
  const before = await page.evaluate(
    ({ channelId, anchorId }) => {
      const client = window.__BUZZ_E2E_QUERY_CLIENT__ as QueryClient;
      const store = client.getQueryData<ChannelWindowStore>([
        "channel-window",
        channelId,
      ]);
      const rows = client.getQueryData<RelayEvent[]>([
        "channel-messages",
        channelId,
      ]);
      if (!store || !rows) throw Error("Missing retained history");
      const anchorIndex = rows.findIndex((row) => row.id === anchorId);
      const olderNeighbor = rows[anchorIndex - 5];
      const newerNeighbor = rows[anchorIndex - 4];
      if (!olderNeighbor || !newerNeighbor) throw Error("No interior gap");
      return {
        ids: rows.map((row) => row.id),
        pages: store.pages.length,
        anchorIndex,
        gapIndex: anchorIndex - 4,
        createdAt: Math.floor(
          (olderNeighbor.created_at + newerNeighbor.created_at) / 2,
        ),
        verifiedPageIds: store.pages[1].rows.map((row) => row.event.id),
      };
    },
    { channelId, anchorId: anchor.id },
  );
  expect(before.pages).toBe(5);
  expect(before.ids.length - before.anchorIndex).toBeGreaterThan(75);
  expect(before.gapIndex).toBeGreaterThan(0);
  expect(before.verifiedPageIds).toContain(before.ids[before.gapIndex]);
  await beginAnchorTrace(page, anchor.id);
  const gap = await page.evaluate((createdAt) => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: { ...window.__BUZZ_E2E__?.mock, channelWindowDelayMs: 150 },
    };
    // Clear the mock sockets synchronously before emitting, so this row cannot
    // reach the UI through live delivery. The reconnect must fetch it.
    window.__BUZZ_E2E_RESTART_MOCK_WEBSOCKETS__?.();
    if (
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "deep-history",
      })
    )
      throw Error("Expected disconnected mock subscription");
    const event = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "deep-history",
      content: "Recovered middle row above the reader\nwith a second line",
      createdAt,
    });
    if (!event) throw Error("Missing injected gap event");
    return event;
  }, before.createdAt);
  await expect
    .poll(() =>
      page.evaluate(
        ({ channelId, gapId }) => {
          const client = window.__BUZZ_E2E_QUERY_CLIENT__ as QueryClient;
          return (
            client
              .getQueryData<ChannelWindowStore>(["channel-window", channelId])
              ?.pages.some((page) =>
                page.rows.some((row) => row.event.id === gapId),
              ) &&
            client.isFetching({ queryKey: ["channel-messages", channelId] }) ===
              0
          );
        },
        { channelId, gapId: gap.id },
      ),
    )
    .toBe(true);
  await expect(
    timeline.locator(`[data-message-id="${gap.id}"]`),
  ).toBeAttached();
  await waitForHistorySettled(page);
  const after = await page.evaluate((channelId) => {
    const client = window.__BUZZ_E2E_QUERY_CLIENT__ as QueryClient;
    return client
      .getQueryData<RelayEvent[]>(["channel-messages", channelId])
      ?.map((row) => row.id);
  }, channelId);
  const expected = before.ids.toSpliced(before.gapIndex, 0, gap.id);
  expect(after).toEqual(expected); // unchanged first/last IDs; not a prepend
  const gapTop = await anchorTop(page, gap.id);
  expect(gapTop).not.toBeNull();
  expect(gapTop ?? Infinity).toBeLessThan(anchor.top);
  await assertAnchorTrace(page, anchor.top);
  const trace = await page.evaluate(
    () =>
      (window as unknown as { __HISTORY_ANCHOR_TRACE__: number[] })
        .__HISTORY_ANCHOR_TRACE__,
  );
  await testInfo.attach("middle-insertion-anchor.json", {
    body: JSON.stringify({
      retainedPages: before.pages,
      retainedRows: before.ids.length,
      insertionIndex: before.gapIndex,
      anchorIndex: before.anchorIndex,
      anchor,
      gapTop,
      samples: trace.length,
      maximumDrift: Math.max(...trace.map((top) => Math.abs(top - anchor.top))),
    }),
    contentType: "application/json",
  });
});

test("DM exhaustion adds intro and date markers without displacing the reading row", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );
  await page.evaluate(() =>
    window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "alice-tyler",
      count: 80,
      lineCount: 2,
    }),
  );
  await page.getByTestId("channel-alice-tyler").click();
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await expect(page.getByTestId("message-dm-intro")).toHaveCount(0);
  await page.evaluate(() => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: { ...window.__BUZZ_E2E__?.mock, channelWindowDelayMs: 600 },
    };
  });
  await startOlderHistory(page);
  // The browser applies wheel scrolling asynchronously. Start the stationary
  // anchor oracle after that input has landed, still before the delayed page.
  await expect
    .poll(() => timeline.evaluate((element) => element.scrollTop))
    .toBe(0);
  const anchor = await visibleAnchor(page);
  await beginAnchorTrace(page, anchor.id);
  await waitForHistorySettled(page);
  await assertAnchorTrace(page, anchor.top);
  await timeline.evaluate((element) => {
    element.scrollTop = 0;
  });
  await expect(page.getByTestId("message-dm-intro")).toBeVisible();
});

test("deletion and live metadata during a prepend preserve the surviving reading row", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(() => {
    for (let index = 0; index < 240; index++)
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "engineering",
        content: `Reading fixture ${index}`,
        id: index.toString(16).padStart(64, "0"),
        createdAt: Math.floor(Date.now() / 1000) - 240 + index,
      });
  });
  await page.getByTestId("channel-engineering").click();
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "engineering",
            kind: 5,
          }) ?? false,
      ),
    )
    .toBe(true);
  await pageOlderHistory(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: { ...window.__BUZZ_E2E__?.mock, channelWindowDelayMs: 800 },
    };
  });
  await startOlderHistory(page);
  await expect
    .poll(() => timeline.evaluate((element) => element.scrollTop))
    .toBe(0);
  const anchor = await visibleAnchor(page);
  const victim = await timeline
    .locator("[data-message-id]")
    .nth(3)
    .getAttribute("data-message-id");
  expect(victim).not.toBe(anchor.id);
  await beginAnchorTrace(page, anchor.id);
  await page.evaluate(
    ({ victim, rootId }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "engineering",
        content: "",
        kind: 5,
        extraTags: [["e", victim]],
      });
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "engineering",
        content: "live output held behind latest",
      });
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "engineering",
        kind: 39005,
        extraTags: [["e", rootId]],
        content: JSON.stringify({
          reply_count: 2,
          descendant_count: 2,
          last_reply_at: Math.floor(Date.now() / 1000),
          participants: [],
        }),
      });
    },
    { victim, rootId: anchor.id },
  );
  await expect(timeline.locator(`[data-message-id="${victim}"]`)).toHaveCount(
    0,
  );
  await waitForHistorySettled(page);
  await assertAnchorTrace(page, anchor.top);
  // Late intrinsic reflow above the resting reader, after scrollend has retired
  // transaction mode. The ordinary Virtua measurement path must also preserve it.
  await timeline.evaluate((element, anchorId) => {
    const anchor = element
      .querySelector(`[data-message-id="${anchorId}"]`)
      ?.closest("[data-timeline-item-key]");
    const rows = [
      ...element.querySelectorAll<HTMLElement>("[data-timeline-item-key]"),
    ];
    const above = rows[rows.indexOf(anchor as HTMLElement) - 1];
    if (!above) throw Error("Expected mounted row above anchor after prepend");
    above.style.paddingBottom = "73px";
  }, anchor.id);
  await page.waitForTimeout(300);
  await assertAnchorTrace(page, anchor.top);
  await pageOlderHistory(page);
});

for (const recovery of [
  "Retry",
  "Load latest",
  "Load unchanged latest",
] as const) {
  test(`failed refresh keeps the reading window and ${recovery} recovers`, async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("channel-deep-history").click();
    const timeline = page.getByTestId("message-timeline");
    await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "deep-history",
            }) ?? false,
        ),
      )
      .toBe(true);
    for (let step = 0; step < 3; step++) await pageOlderHistory(page);
    const anchor = await visibleAnchor(page);
    await beginAnchorTrace(page, anchor.id);
    await page.evaluate((recovery) => {
      const w = window as typeof window & {
        __FAIL_HISTORY_REFRESH__?: boolean;
        __TAURI_INTERNALS__: {
          invoke: (
            command: string,
            payload: unknown,
            options: unknown,
          ) => Promise<unknown>;
        };
      };
      w.__FAIL_HISTORY_REFRESH__ = true;
      const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
      w.__TAURI_INTERNALS__.invoke = (command, payload, options) => {
        if (command === "get_channel_window" && w.__FAIL_HISTORY_REFRESH__)
          return Promise.reject(new Error("forced history refresh failure"));
        return original(command, payload, options);
      };
      if (recovery !== "Load unchanged latest")
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "deep-history",
          content: "latest refresh recovery row",
          createdAt: Math.floor(Date.now() / 1000) + 1,
        });
      window.__BUZZ_E2E_RESTART_MOCK_WEBSOCKETS__?.();
    }, recovery);
    const notice = page.getByTestId("history-refresh-error");
    await expect(notice).toContainText(
      "Your loaded history is still available.",
    );
    // A second failure must remain visible/retryable without discarding rows.
    await notice
      .getByRole("button", {
        name: recovery === "Retry" ? "Retry" : "Load latest",
        exact: true,
      })
      .click();
    await expect(notice).toBeVisible();
    // Observe a full request cycle, not the enabled render before invalidation.
    await expect(
      notice.getByRole("button", { name: "Retry", exact: true }),
    ).toBeDisabled();
    // Keep the failure switch on through Query's automatic retry. Otherwise
    // it can recover and remove the banner before the next explicit click.
    await expect(
      notice.getByRole("button", { name: "Retry", exact: true }),
    ).toBeEnabled();
    await assertAnchorTrace(page, anchor.top);
    expect(await anchorTop(page, anchor.id)).not.toBeNull();
    await page.evaluate(() => {
      (
        window as typeof window & { __FAIL_HISTORY_REFRESH__?: boolean }
      ).__FAIL_HISTORY_REFRESH__ = false;
    });
    await notice
      .getByRole("button", {
        name: recovery === "Retry" ? "Retry" : "Load latest",
        exact: true,
      })
      .click();
    await expect(notice).toHaveCount(0);
    await waitForHistorySettled(page);
    if (recovery === "Retry") {
      await assertAnchorTrace(page, anchor.top);
      await pageOlderHistory(page);
    } else {
      if (recovery !== "Load unchanged latest")
        await expect(timeline).toContainText("latest refresh recovery row");
      await expect
        .poll(() =>
          timeline.evaluate((element) =>
            Math.abs(
              element.scrollHeight - element.clientHeight - element.scrollTop,
            ),
          ),
        )
        .toBeLessThan(5);
      // Explicit replacement retires the old transaction; paging remains usable.
      await pageOlderHistory(page);
    }
  });
}
