import { expect, test } from "@playwright/test";
import type { Locator, Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const FIRST_PASS_PREPEND_DRIFT_PX = 20;
const LATE_REFLOW_DRIFT_PX = 4;

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

test("timeline reserves mixed-media rows before fast scrollback", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(45_000);
  const noDimImageUrl = "https://example.com/e2e/timeline-mixed-no-dim.png";

  await installMockBridge(page);
  await page.route(noDimImageUrl, (route) => {
    route.fulfill({
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240" viewBox="0 0 320 240"><rect width="100%" height="100%" fill="#7c3aed"/></svg>',
      contentType: "image/svg+xml",
    });
  });
  await page.goto("/");
  await waitForMockTimelineBridge(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E__ = {
      ...window.__BUZZ_E2E__,
      mock: { ...window.__BUZZ_E2E__?.mock, historyDelayMs: 10_000 },
    };
  });
  await page.evaluate((imageUrl) => {
    for (let index = 0; index < 18; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `mixed-scroll lead-in ${index}\nplain lead-in line ${index}`,
        createdAt: 1_700_600_000 + index,
      });
    }
    const dimmedUrl =
      "http://localhost:3000/media/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee.png";
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: ["mixed-scroll dimmed media", `![dimmed](${dimmedUrl})`].join(
        "\n",
      ),
      extraTags: [
        [
          "imeta",
          `url ${dimmedUrl}`,
          "m image/png",
          "x eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
          "size 1234",
          "dim 320x180",
          "filename mixed-dimmed.png",
        ],
      ],
      createdAt: 1_700_600_100,
    });
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: ["mixed-scroll no-dim media", `![no dim](${imageUrl})`].join(
        "\n",
      ),
      createdAt: 1_700_600_101,
    });
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: [
        "mixed-scroll fenced code",
        "```ts",
        "function teleportProbe(input: string) {",
        "  const rows = input.split(/\\n/);",
        "  return rows.map((row, index) => ({ row, index }));",
        "}",
        "const result = teleportProbe('one\\ntwo\\nthree');",
        "console.log(result);",
        "console.log(result.length);",
        "```",
      ].join("\n"),
      createdAt: 1_700_600_102,
    });
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: [
        "mixed-scroll link preview",
        "https://github.com/block/sprout/pull/1334",
      ].join("\n"),
      createdAt: 1_700_600_103,
    });
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: [
        "mixed-scroll tall text",
        "alpha beta gamma delta epsilon zeta eta theta iota kappa",
        "lambda mu nu xi omicron pi rho sigma tau upsilon",
        "phi chi psi omega plus extra words to wrap across lines",
        "another paragraph with enough text to create a tall row",
        "final tall text line for the intrinsic height estimate",
      ].join("\n"),
      createdAt: 1_700_600_104,
    });
    for (let index = 0; index < 36; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `mixed-scroll tail ${index}\nplain tail line ${index}`,
        createdAt: 1_700_600_200 + index,
      });
    }
  }, noDimImageUrl);

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-timeline"]',
    ) as HTMLDivElement | null;
    return element && element.scrollHeight > element.clientHeight + 2_000;
  });

  const intrinsic = (rowText: string) =>
    timeline
      .locator("[data-message-id]")
      .filter({ hasText: rowText })
      .first()
      .evaluate((row) => {
        const scroller = row.closest('[data-testid="message-timeline"]');
        let node: HTMLElement | null = row.parentElement;
        while (node && node !== scroller) {
          if (node.classList.contains("timeline-row-cv")) {
            const match = getComputedStyle(node).containIntrinsicSize.match(
              /(?:auto\s+)?([0-9.]+)px/,
            );
            return match ? Number(match[1]) : 0;
          }
          node = node.parentElement;
        }
        return 0;
      });

  await expect
    .poll(() => intrinsic("mixed-scroll dimmed media"))
    .toBeGreaterThan(120);
  await expect
    .poll(() => intrinsic("mixed-scroll no-dim media"))
    .toBeGreaterThan(180);
  await expect
    .poll(() => intrinsic("mixed-scroll fenced code"))
    .toBeGreaterThan(160);
  await expect
    .poll(() => intrinsic("mixed-scroll link preview"))
    .toBeGreaterThan(110);
  await expect
    .poll(() => intrinsic("mixed-scroll tall text"))
    .toBeGreaterThan(120);

  const anchorId = await timeline
    .locator("[data-message-id]")
    .filter({ hasText: "mixed-scroll tail 28" })
    .first()
    .evaluate((row) => row.dataset.messageId ?? "");
  expect(anchorId).not.toBe("");

  const drift = await timeline.evaluate(async (element, id) => {
    const scroller = element as HTMLDivElement;
    const anchor = scroller.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(id)}"]`,
    );
    if (!anchor) throw new Error("mixed-scroll anchor row missing");
    const currentTop =
      anchor.getBoundingClientRect().top - scroller.getBoundingClientRect().top;
    scroller.scrollTop += currentTop - 280;
    scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
    let baseline: number | null = null;
    let maxDrift = 0;
    let missingSamples = 0;
    let samples = 0;
    const contentTop = () => {
      const row = scroller.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(id)}"]`,
      );
      if (!row) return null;
      return (
        row.getBoundingClientRect().top -
        scroller.getBoundingClientRect().top +
        scroller.scrollTop
      );
    };
    const sample = () => {
      const top = contentTop();
      if (top === null) missingSamples += 1;
      else {
        if (baseline === null) baseline = top;
        maxDrift = Math.max(maxDrift, Math.abs(top - baseline));
      }
      samples += 1;
    };
    sample();
    // Stop before the top-edge history loader takes over; this case is scoped
    // to rows realizing during fast mixed-media scrollback, not prepend anchoring.
    for (let frame = 0; frame < 24 && scroller.scrollTop > 0; frame += 1) {
      const PX = 95;
      scroller.dispatchEvent(
        new WheelEvent("wheel", {
          bubbles: true,
          cancelable: true,
          deltaY: -PX,
        }),
      );
      scroller.scrollTop = Math.max(0, scroller.scrollTop - PX);
      scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
      await new Promise<void>((r) => requestAnimationFrame(() => r()));
      sample();
    }
    await new Promise<void>((r) => setTimeout(r, 250));
    sample();
    return {
      baseline,
      maxDrift,
      missingSamples,
      samples,
      scrollTop: scroller.scrollTop,
    };
  }, anchorId);

  console.info("timeline-mixed-media-scrollback result", JSON.stringify(drift));
  expect(drift.samples).toBeGreaterThan(0);
  expect(drift.missingSamples).toBe(0);
  expect(drift.maxDrift).toBeLessThanOrEqual(120);
});

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
  await expect
    .poll(async () => (await snapshotAnchor(timeline)).oldestOlderIndex, {
      timeout: 8_000,
    })
    .not.toBeNull();

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
    // First-pass prepended rows realize from content-visibility estimates to
    // true height over a few frames. Keep this bound tight enough to catch the
    // old teleport class; a measured-height cache is the future lever for
    // sub-2px first-pass precision.
  ).toBeLessThanOrEqual(FIRST_PASS_PREPEND_DRIFT_PX);
  expectAnchorOrderUnchanged(before, afterPrepend);

  const prependDrift = await stopAnchorDriftSampler(timeline);
  console.info(
    "timeline-prepend-no-shift result",
    JSON.stringify(prependDrift),
  );
  expect(prependDrift.samples).toBeGreaterThan(0);
  expect(prependDrift.missingSamples).toBe(0);
  expect(prependDrift.maxDrift).toBeLessThanOrEqual(
    FIRST_PASS_PREPEND_DRIFT_PX,
  );

  await startAnchorDriftSampler(
    timeline,
    before.anchorId,
    afterPrepend.anchorTop,
  );

  const grew = await growRowAboveAnchor(timeline, before.anchorId);
  expect(grew).toBe(true);

  await expect
    .poll(
      async () => {
        const top = await getAnchorTop(timeline, before.anchorId);
        return top === null
          ? Number.POSITIVE_INFINITY
          : Math.abs(top - afterPrepend.anchorTop);
      },
      { timeout: 3_000 },
    )
    .toBeLessThanOrEqual(LATE_REFLOW_DRIFT_PX);

  const afterReflow = await snapshotAnchor(timeline);
  expect(afterReflow.anchorId).toBe(before.anchorId);
  expectAnchorOrderUnchanged(before, afterReflow);

  const drift = await stopAnchorDriftSampler(timeline);
  console.info("timeline-reflow-no-shift result", JSON.stringify(drift));
  expect(drift.samples).toBeGreaterThan(0);
  expect(drift.missingSamples).toBe(0);
  expect(drift.maxDrift).toBeLessThanOrEqual(LATE_REFLOW_DRIFT_PX);
});

test("de-virtualized timeline rows apply content-visibility", async ({
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

  // Guards against a typo'd/removed wrapper class shipping inert — a bad
  // utility name is invisible to typecheck.
  const hasContentVisibility = await timeline
    .locator("[data-message-id]")
    .first()
    .evaluate((element) => {
      const scroller = element.closest('[data-testid="message-timeline"]');
      let node: HTMLElement | null = element.parentElement;
      while (node && node !== scroller) {
        if (getComputedStyle(node).contentVisibility === "auto") return true;
        node = node.parentElement;
      }
      return false;
    });
  expect(hasContentVisibility).toBe(true);
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

test("thread panel stays put while replies stream in mid-scroll", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(45_000);

  await installMockBridge(page);
  await page.goto("/");
  await waitForMockTimelineBridge(page);

  page.on("console", (msg) => {
    if (msg.text().includes("ANCHOR_DEBUG")) console.info(msg.text());
  });

  const rootId = await page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "thread stream root",
      createdAt: 1_700_400_000,
    });
    if (!root) throw new Error("Failed to seed thread stream root");

    for (let index = 0; index < 48; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `thread stream reply ${index}\nsecond line ${index}\nthird line ${index}`,
        parentEventId: root.id,
        createdAt: 1_700_400_001 + index,
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
  const threadBody = threadPanel.getByTestId("message-thread-body");
  await expect(threadBody.locator("[data-message-id]").first()).toBeVisible();
  await page.waitForFunction(() => {
    const element = document.querySelector(
      '[data-testid="message-thread-body"]',
    ) as HTMLDivElement | null;
    return element && element.scrollHeight > element.clientHeight + 800;
  });

  // Park the reader mid-history (a real "reading older replies" position), not
  // at the bottom — the at-bottom stick path corrects differently.
  await threadBody.evaluate((element) => {
    const scroller = element as HTMLDivElement;
    scroller.scrollTop = Math.floor(scroller.scrollHeight * 0.4);
    scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  await page.waitForTimeout(100);

  const before = await snapshotAnchor(threadBody);
  expect(before.anchorId).not.toBe("");

  // Scroll-compensated drift probe: a row's position in *content* coordinates
  // (viewport-top offset + scrollTop) is invariant unless content ABOVE it
  // changes height. This isolates involuntary jumps (the bug) from the
  // reader's own deliberate scroll, which the shared viewport-top sampler
  // cannot distinguish.
  await threadBody.evaluate((element, anchorId) => {
    const scroller = element as HTMLDivElement;
    const win = window as typeof window & {
      __THREAD_STREAM_PROBE__?: {
        stop: boolean;
        maxDrift: number;
        samples: number;
        baseline: number | null;
      };
    };
    const probe = { stop: false, maxDrift: 0, samples: 0, baseline: null };
    win.__THREAD_STREAM_PROBE__ = probe;
    const contentTop = () => {
      const row = scroller.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(anchorId)}"]`,
      );
      if (!row) return null;
      return (
        row.getBoundingClientRect().top -
        scroller.getBoundingClientRect().top +
        scroller.scrollTop
      );
    };
    const sample = () => {
      if (probe.stop) return;
      const top = contentTop();
      if (top !== null) {
        if (probe.baseline === null) probe.baseline = top;
        probe.maxDrift = Math.max(
          probe.maxDrift,
          Math.abs(top - probe.baseline),
        );
      }
      probe.samples += 1;
      requestAnimationFrame(sample);
    };
    requestAnimationFrame(sample);
  }, before.anchorId);

  // Drive a brisk upward wheel scroll while new replies stream into the open
  // thread on the live-event path — Wes's exact symptom: scrolling a live
  // thread while replies arrive. The reading row's CONTENT position must hold;
  // the reader's scroll is compensated out, so any drift here is the bug.
  for (let batch = 0; batch < 6; batch += 1) {
    await threadBody.evaluate((element) => {
      const scroller = element as HTMLDivElement;
      const PX = 40;
      scroller.dispatchEvent(
        new WheelEvent("wheel", {
          deltaY: -PX,
          bubbles: true,
          cancelable: true,
        }),
      );
      scroller.scrollTop = Math.max(0, scroller.scrollTop - PX);
      scroller.dispatchEvent(new Event("scroll", { bubbles: true }));
    });
    await page.evaluate(
      ({ rootEventId, index }) => {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: `thread stream live reply ${index}\nsecond line ${index}`,
          parentEventId: rootEventId,
          createdAt: 1_700_400_100 + index,
        });
      },
      { rootEventId: rootId, index: batch },
    );
    await page.waitForTimeout(80);
  }

  await page.waitForTimeout(300);

  const drift = await threadBody.evaluate((element) => {
    const win = window as typeof window & {
      __THREAD_STREAM_PROBE__?: {
        stop: boolean;
        maxDrift: number;
        samples: number;
      };
    };
    const probe = win.__THREAD_STREAM_PROBE__;
    if (!probe) throw new Error("no thread stream probe installed");
    probe.stop = true;
    return {
      maxDrift: probe.maxDrift,
      samples: probe.samples,
      scrollTop: (element as HTMLDivElement).scrollTop,
    };
  });
  console.info("thread-panel-stream-scroll result", JSON.stringify(drift));
  expect(drift.samples).toBeGreaterThan(0);
  // The reader is driving the scroll; streamed replies append below the reading
  // row. In content coordinates the row must not move. Any material drift here
  // is the "content jumps around like crazy" bug.
  expect(drift.maxDrift).toBeLessThanOrEqual(2);
});
