import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

type AnchorSnapshot = {
  anchorId: string;
  anchorTop: number;
  visibleIds: string[];
  rowsFromAnchor: string[];
  oldestOlderIndex: number | null;
  scrollHeight: number;
};

async function waitForMockTimelineBridge(page: Page) {
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__ === "function",
  );
}

async function seedNoShiftTimeline(page: Page) {
  await page.evaluate(() => {
    for (let index = 0; index < 40; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `no-shift current ${index}\nsecond line ${index}\nthird line ${index}`,
        createdAt: 1_700_100_000 + index,
      });
    }

    window.__BUZZ_E2E_PREPEND_MOCK_HISTORY__?.({
      channelName: "general",
      count: 600,
      lineCount: 3,
      createdAtStart: 1_700_000_000,
    });
  });
}

async function snapshotAnchor(timeline: Locator): Promise<AnchorSnapshot> {
  return timeline.evaluate((element) => {
    const scroller = element as HTMLDivElement;
    const scrollerRect = scroller.getBoundingClientRect();
    const rows = Array.from(
      scroller.querySelectorAll<HTMLElement>("[data-message-id]"),
    );
    const visibleRows = rows.filter((row) => {
      const rect = row.getBoundingClientRect();
      return rect.bottom > scrollerRect.top && rect.top < scrollerRect.bottom;
    });
    const anchor =
      visibleRows.find((row) => {
        const rect = row.getBoundingClientRect();
        return rect.top - scrollerRect.top >= 0;
      }) ?? visibleRows[0];

    if (!anchor) {
      throw new Error("no visible message row to anchor");
    }

    const anchorIndex = rows.indexOf(anchor);
    const oldestOlderIndex = rows.reduce<number | null>((oldest, row) => {
      const match = row.textContent?.match(/mock older (\d+) line/);
      if (!match) return oldest;
      const index = Number(match[1]);
      return oldest === null ? index : Math.min(oldest, index);
    }, null);

    return {
      anchorId: anchor.dataset.messageId ?? "",
      anchorTop: anchor.getBoundingClientRect().top - scrollerRect.top,
      visibleIds: visibleRows
        .map((row) => row.dataset.messageId ?? "")
        .filter(Boolean),
      rowsFromAnchor: rows
        .slice(anchorIndex, anchorIndex + 5)
        .map((row) => row.dataset.messageId ?? "")
        .filter(Boolean),
      oldestOlderIndex,
      scrollHeight: scroller.scrollHeight,
    };
  });
}

async function getAnchorTop(
  timeline: Locator,
  anchorId: string,
): Promise<number | null> {
  return timeline.evaluate((element, id) => {
    const scroller = element as HTMLDivElement;
    const anchor = scroller.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(id)}"]`,
    );
    if (!anchor) return null;
    return (
      anchor.getBoundingClientRect().top - scroller.getBoundingClientRect().top
    );
  }, anchorId);
}

async function startAnchorDriftSampler(
  timeline: Locator,
  anchorId: string,
  baselineTop: number,
) {
  await timeline.evaluate(
    (element, input) => {
      const scroller = element as HTMLDivElement;
      const win = window as typeof window & {
        __TIMELINE_NO_SHIFT_PROBE__?: {
          stop: boolean;
          maxDrift: number;
          missingSamples: number;
          samples: number;
        };
      };
      const probe = {
        stop: false,
        maxDrift: 0,
        missingSamples: 0,
        samples: 0,
      };
      win.__TIMELINE_NO_SHIFT_PROBE__ = probe;

      const sample = () => {
        if (probe.stop) return;
        const anchor = scroller.querySelector<HTMLElement>(
          `[data-message-id="${CSS.escape(input.anchorId)}"]`,
        );
        if (!anchor) {
          probe.missingSamples += 1;
        } else {
          const top =
            anchor.getBoundingClientRect().top -
            scroller.getBoundingClientRect().top;
          probe.maxDrift = Math.max(
            probe.maxDrift,
            Math.abs(top - input.baselineTop),
          );
        }
        probe.samples += 1;
        requestAnimationFrame(sample);
      };

      requestAnimationFrame(sample);
    },
    { anchorId, baselineTop },
  );
}

async function stopAnchorDriftSampler(timeline: Locator) {
  return timeline.evaluate((element) => {
    const win = window as typeof window & {
      __TIMELINE_NO_SHIFT_PROBE__?: {
        stop: boolean;
        maxDrift: number;
        missingSamples: number;
        samples: number;
      };
    };
    const probe = win.__TIMELINE_NO_SHIFT_PROBE__;
    if (!probe) throw new Error("no timeline no-shift sampler installed");
    probe.stop = true;
    return {
      maxDrift: probe.maxDrift,
      missingSamples: probe.missingSamples,
      samples: probe.samples,
      scrollTop: (element as HTMLDivElement).scrollTop,
    };
  });
}

async function growRowAboveAnchor(timeline: Locator, anchorId: string) {
  return timeline.evaluate((element, id) => {
    const scroller = element as HTMLDivElement;
    const rows = Array.from(
      scroller.querySelectorAll<HTMLElement>("[data-message-id]"),
    );
    const anchorIndex = rows.findIndex((row) => row.dataset.messageId === id);
    if (anchorIndex <= 0) return false;

    const target = rows[Math.max(0, anchorIndex - 2)];
    if (!target) return false;

    const currentHeight = target.getBoundingClientRect().height;
    target.style.minHeight = `${currentHeight + 96}px`;
    target.dataset.noShiftReflowTarget = "true";
    return true;
  }, anchorId);
}

function expectAnchorOrderUnchanged(
  before: AnchorSnapshot,
  after: AnchorSnapshot,
) {
  expect(after.visibleIds).toContain(before.anchorId);
  expect(after.rowsFromAnchor).toEqual(before.rowsFromAnchor);
}

test("timeline prepend plus late row reflow keeps the reading row stable", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(45_000);

  await installMockBridge(page);
  await page.goto("/");
  await waitForMockTimelineBridge(page);
  await seedNoShiftTimeline(page);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return element && element.scrollHeight > element.clientHeight + 1_000;
  });

  await page.evaluate(() => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: { ...window.__BUZZ_E2E__?.mock, historyDelayMs: 1_000 },
    };
    (
      window as unknown as { __HISTORY_INFLIGHT__?: number }
    ).__HISTORY_INFLIGHT__ = 0;
  });

  await timeline.evaluate((element) => {
    const scroller = element as HTMLDivElement;
    scroller.scrollTop = 150;
    scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
  });

  await expect
    .poll(
      async () =>
        page.evaluate(
          () =>
            (window as unknown as { __HISTORY_INFLIGHT__?: number })
              .__HISTORY_INFLIGHT__ ?? 0,
        ),
      { timeout: 5_000 },
    )
    .toBeGreaterThan(0);

  const before = await snapshotAnchor(timeline);
  expect(before.anchorId).not.toBe("");
  expect(before.oldestOlderIndex).not.toBeNull();
  await startAnchorDriftSampler(timeline, before.anchorId, before.anchorTop);

  await expect
    .poll(
      async () => {
        const snapshot = await snapshotAnchor(timeline);
        return snapshot.oldestOlderIndex !== null &&
          before.oldestOlderIndex !== null &&
          snapshot.oldestOlderIndex < before.oldestOlderIndex &&
          snapshot.scrollHeight > before.scrollHeight + 1_000
          ? "landed"
          : "pending";
      },
      { timeout: 6_000 },
    )
    .toBe("landed");

  const afterPrepend = await snapshotAnchor(timeline);
  expect(afterPrepend.anchorId).toBe(before.anchorId);
  expect(
    Math.abs(afterPrepend.anchorTop - before.anchorTop),
  ).toBeLessThanOrEqual(2);
  expectAnchorOrderUnchanged(before, afterPrepend);

  const grew = await growRowAboveAnchor(timeline, before.anchorId);
  expect(grew).toBe(true);

  await expect
    .poll(
      async () => {
        const top = await getAnchorTop(timeline, before.anchorId);
        return top === null
          ? Number.POSITIVE_INFINITY
          : Math.abs(top - before.anchorTop);
      },
      { timeout: 3_000 },
    )
    .toBeLessThanOrEqual(2);

  const afterReflow = await snapshotAnchor(timeline);
  expect(afterReflow.anchorId).toBe(before.anchorId);
  expectAnchorOrderUnchanged(before, afterReflow);

  const drift = await stopAnchorDriftSampler(timeline);
  console.info("timeline-no-shift result", JSON.stringify(drift));
  expect(drift.samples).toBeGreaterThan(0);
  expect(drift.missingSamples).toBe(0);
  expect(drift.maxDrift).toBeLessThanOrEqual(2);
});

test("thread panel late row reflow keeps the reading reply stable", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(45_000);

  await installMockBridge(page);
  await page.goto("/");
  await waitForMockTimelineBridge(page);

  const rootId = await page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "thread no-shift root",
      createdAt: 1_700_300_000,
    });
    if (!root) throw new Error("Failed to seed thread no-shift root");

    for (let index = 0; index < 48; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `thread no-shift reply ${index}\nsecond line ${index}\nthird line ${index}`,
        parentEventId: root.id,
        createdAt: 1_700_300_001 + index,
      });
    }

    return root.id;
  });

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  const summary = timeline.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible({ timeout: 5_000 });
  await summary.click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "thread no-shift root",
  );

  const threadBody = threadPanel.getByTestId("message-thread-body");
  await expect(threadBody.locator("[data-message-id]").first()).toBeVisible();
  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-thread-body"]',
    ) as HTMLDivElement | null;
    return element && element.scrollHeight > element.clientHeight + 800;
  });

  await threadBody.evaluate((element) => {
    const scroller = element as HTMLDivElement;
    scroller.scrollTop = Math.floor(scroller.scrollHeight / 2);
    scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(100);

  const before = await snapshotAnchor(threadBody);
  expect(before.anchorId).not.toBe("");
  await startAnchorDriftSampler(threadBody, before.anchorId, before.anchorTop);

  const grew = await growRowAboveAnchor(threadBody, before.anchorId);
  expect(grew).toBe(true);

  await expect
    .poll(
      async () => {
        const top = await getAnchorTop(threadBody, before.anchorId);
        return top === null
          ? Number.POSITIVE_INFINITY
          : Math.abs(top - before.anchorTop);
      },
      { timeout: 3_000 },
    )
    .toBeLessThanOrEqual(2);

  await page.waitForTimeout(500);

  const afterReflow = await snapshotAnchor(threadBody);
  expect(afterReflow.anchorId).toBe(before.anchorId);
  expectAnchorOrderUnchanged(before, afterReflow);

  const drift = await stopAnchorDriftSampler(threadBody);
  console.info("thread-panel-no-shift result", JSON.stringify(drift));
  expect(drift.samples).toBeGreaterThan(0);
  expect(drift.missingSamples).toBe(0);
  expect(drift.maxDrift).toBeLessThanOrEqual(2);
});
