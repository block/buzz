import { expect, test, type Page } from "@playwright/test";

import { TEST_IDENTITIES, installMockBridge } from "../helpers/bridge";
import { logMeasurement, measureAction } from "./perf/metrics";

const BUSY_ROWS = 220;
const THREAD_REPLIES = 24;
const TYPING_SAMPLE =
  "Drafting a latency repro with @alice, a second paragraph, and enough text to exercise wrapping in the composer.";

type MockMessageEvent = { id: string; created_at: number; pubkey: string };

async function waitForMockHooks(page: Page) {
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      typeof window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__ === "function",
  );
}

async function emitMockMessage(
  page: Page,
  channelName: string,
  content: string,
  options?: {
    createdAt?: number;
    parentEventId?: string;
    pubkey?: string;
  },
): Promise<MockMessageEvent> {
  const event = await page.evaluate(
    ({ channelName: ch, content: body, createdAt, parentEventId, pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: body,
        createdAt,
        parentEventId,
        pubkey,
      }),
    {
      channelName,
      content,
      createdAt: options?.createdAt,
      parentEventId: options?.parentEventId,
      pubkey: options?.pubkey ?? TEST_IDENTITIES.alice.pubkey,
    },
  );
  if (!event) throw new Error("Mock message emitter is not installed");
  return event;
}

async function seedBusyChannel(
  page: Page,
  channelName: string,
  rows = BUSY_ROWS,
) {
  await page.evaluate(
    ({ channelName: ch, rows }) => {
      for (let i = 0; i < rows; i += 1) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: ch,
          content: `latency seed ${ch} row ${i}\nsecond line to exercise wrapping`,
        });
      }
    },
    { channelName, rows },
  );
}

async function clickChannel(page: Page, channelName: string) {
  const channel = page.getByTestId(`channel-${channelName}`);
  await expect(page.getByTestId("app-sidebar")).toBeVisible();
  await expect(channel).toBeVisible();

  // The sidebar can briefly re-render while mock live messages update unread
  // state. Keep setup resilient by retrying the locator, which re-resolves on
  // each attempt instead of holding a stale DOM node.
  let lastError: unknown;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      await channel.click({ timeout: 5_000 });
      return;
    } catch (error) {
      lastError = error;
      await page.waitForTimeout(100);
      await expect(channel).toBeVisible();
    }
  }

  throw lastError;
}

async function openChannel(page: Page, channelName: string) {
  await clickChannel(page, channelName);
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  await expect(page.getByTestId("message-row").first()).toBeVisible();
}

async function mountedRowCount(page: Page) {
  return page.getByTestId("message-row").count();
}

test.describe("Buzz GUI latency harness", () => {
  test("MEASURE: composer typing latency in a busy channel", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await waitForMockHooks(page);
    await seedBusyChannel(page, "general");
    await openChannel(page, "general");

    const input = page.getByTestId("message-input");
    await expect(input).toBeVisible();

    const measurement = await measureAction(page, async () => {
      await input.click();
      await page.keyboard.type(TYPING_SAMPLE, { delay: 0 });
      await expect(input).toContainText("latency repro");
      return {
        chars: TYPING_SAMPLE.length,
        rows: await mountedRowCount(page),
      };
    });

    logMeasurement("COMPOSER TYPING LATENCY (busy channel, no send)", {
      "chars typed": measurement.result.chars,
      "rows mounted": measurement.result.rows,
      "wall time": `${measurement.wallMs.toFixed(1)}ms`,
      "ms / char": (measurement.wallMs / measurement.result.chars).toFixed(2),
      "layout time": `${measurement.metrics.layoutMs.toFixed(1)}ms`,
      "style recalc": `${measurement.metrics.recalcMs.toFixed(1)}ms`,
      "script time": `${measurement.metrics.scriptMs.toFixed(1)}ms`,
      "task time": `${measurement.metrics.taskMs.toFixed(1)}ms`,
      "layout count": measurement.metrics.layoutCount,
    });

    expect(measurement.result.chars).toBeGreaterThan(80);
    expect(measurement.result.rows).toBeGreaterThan(50);
    expect(measurement.wallMs).toBeGreaterThan(0);
  });

  test("MEASURE: channel switch latency across seeded busy channels", async ({
    page,
  }) => {
    await installMockBridge(page, { historyDelayMs: 120 });
    await page.goto("/");
    await waitForMockHooks(page);
    await seedBusyChannel(page, "general", 160);
    await seedBusyChannel(page, "engineering", 160);
    await openChannel(page, "general");

    const measurement = await measureAction(page, async () => {
      await clickChannel(page, "engineering");
      await expect(page.getByTestId("chat-title")).toHaveText("engineering");
      await expect(page.getByTestId("message-row").first()).toBeVisible();
      return { rows: await mountedRowCount(page) };
    });

    logMeasurement("CHANNEL SWITCH LATENCY (general → engineering)", {
      "rows mounted": measurement.result.rows,
      "wall time": `${measurement.wallMs.toFixed(1)}ms`,
      "layout time": `${measurement.metrics.layoutMs.toFixed(1)}ms`,
      "style recalc": `${measurement.metrics.recalcMs.toFixed(1)}ms`,
      "script time": `${measurement.metrics.scriptMs.toFixed(1)}ms`,
      "task time": `${measurement.metrics.taskMs.toFixed(1)}ms`,
      "layout count": measurement.metrics.layoutCount,
    });

    expect(measurement.result.rows).toBeGreaterThan(20);
    expect(measurement.wallMs).toBeGreaterThan(0);
  });

  test("MEASURE: thread panel open latency with seeded replies", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await waitForMockHooks(page);
    await openChannel(page, "general");

    for (let i = 0; i < THREAD_REPLIES; i += 1) {
      await emitMockMessage(page, "general", `thread latency reply ${i + 1}`, {
        parentEventId: "mock-general-welcome",
        pubkey: TEST_IDENTITIES.alice.pubkey,
      });
    }

    const threadSummary = page.getByTestId("message-thread-summary").first();
    await expect(threadSummary).toBeVisible();

    const measurement = await measureAction(page, async () => {
      await threadSummary.click();
      const panel = page.getByTestId("message-thread-panel");
      await expect(panel).toBeVisible();
      await expect(
        panel.getByText("thread latency reply 1", { exact: true }),
      ).toBeVisible();
      return {
        replies: await panel.getByTestId("message-row").count(),
      };
    });

    logMeasurement("THREAD OPEN LATENCY (seeded replies)", {
      "replies rendered": measurement.result.replies,
      "wall time": `${measurement.wallMs.toFixed(1)}ms`,
      "layout time": `${measurement.metrics.layoutMs.toFixed(1)}ms`,
      "style recalc": `${measurement.metrics.recalcMs.toFixed(1)}ms`,
      "script time": `${measurement.metrics.scriptMs.toFixed(1)}ms`,
      "task time": `${measurement.metrics.taskMs.toFixed(1)}ms`,
      "layout count": measurement.metrics.layoutCount,
    });

    expect(measurement.result.replies).toBeGreaterThan(5);
    expect(measurement.wallMs).toBeGreaterThan(0);
  });

  test("MEASURE: member search latency in add-people sidebar", async ({
    page,
  }) => {
    await installMockBridge(page, { userSearchDelayMs: 180 });
    await page.goto("/");
    await waitForMockHooks(page);
    await openChannel(page, "general");
    await page.getByTestId("channel-members-trigger").click();
    await expect(page.getByTestId("members-sidebar")).toBeVisible();

    const search = page.getByTestId("channel-management-search-users");
    await expect(search).toBeVisible();

    const measurement = await measureAction(page, async () => {
      await search.fill("outsider");
      const result = page
        .locator('[data-testid^="channel-user-search-result-"]')
        .first();
      await expect(result).toBeVisible();
      return {
        resultText: ((await result.textContent()) ?? "").trim(),
      };
    });

    logMeasurement("ADD-PEOPLE SEARCH LATENCY (read-only)", {
      "first result": measurement.result.resultText,
      "wall time": `${measurement.wallMs.toFixed(1)}ms`,
      "layout time": `${measurement.metrics.layoutMs.toFixed(1)}ms`,
      "style recalc": `${measurement.metrics.recalcMs.toFixed(1)}ms`,
      "script time": `${measurement.metrics.scriptMs.toFixed(1)}ms`,
      "task time": `${measurement.metrics.taskMs.toFixed(1)}ms`,
      "layout count": measurement.metrics.layoutCount,
    });

    expect(measurement.result.resultText.toLowerCase()).toContain("outsider");
    expect(measurement.wallMs).toBeGreaterThan(0);
  });
});
