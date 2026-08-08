import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Focused Wave-1 typing benchmark (quiet vs agent-busy only).
 *
 * Used for before/after proof of ChannelScreen/ChannelPane typing-isolation
 * work. Emits one parseable WAVE1_PERF line per successful iteration.
 */
const THROTTLE_RATE = 4;
const TYPED_TEXT =
  "The quick brown fox jumps over the lazy dog while agents keep working away";
const KEY_DELAY_MS = 60;
const AGENT_COUNT = 8;
const TYPING_EMIT_INTERVAL_MS = 250;
const LIVE_MESSAGE_INTERVAL_MS = 2000;

type LatencyReport = {
  count: number;
  median: number;
  p95: number;
  max: number;
  over50: number;
  longtaskTotal: number;
};

async function resetWindowMetrics(page: import("@playwright/test").Page) {
  await page.evaluate(() => {
    const store = window as unknown as {
      __INPUT_EVENTS__: number[];
      __LONGTASKS__: number[];
    };
    store.__INPUT_EVENTS__ = [];
    store.__LONGTASKS__ = [];
  });
}

async function readWindowMetrics(
  page: import("@playwright/test").Page,
): Promise<LatencyReport> {
  return page.evaluate(() => {
    const store = window as unknown as {
      __INPUT_EVENTS__: number[];
      __LONGTASKS__: number[];
    };
    const durations = [...(store.__INPUT_EVENTS__ ?? [])].sort((a, b) => a - b);
    const at = (q: number) =>
      durations.length === 0
        ? 0
        : durations[
            Math.min(
              durations.length - 1,
              Math.floor(q * (durations.length - 1)),
            )
          ];
    return {
      count: durations.length,
      median: at(0.5),
      p95: at(0.95),
      max: durations.length ? durations[durations.length - 1] : 0,
      over50: durations.filter((d) => d > 50).length,
      longtaskTotal: (store.__LONGTASKS__ ?? []).reduce((s, d) => s + d, 0),
    };
  });
}

async function typeBurst(page: import("@playwright/test").Page) {
  const input = page.getByTestId("message-input").last();
  await expect(input).toBeVisible({ timeout: 30_000 });
  await input.click();
  await input.pressSequentially(TYPED_TEXT, { delay: KEY_DELAY_MS });
  await page.waitForTimeout(500);
  await input.press("Meta+A");
  await input.press("Backspace");
  await page.waitForTimeout(300);
}

test("MEASURE: wave1 typing core quiet vs busy", async ({ page }, testInfo) => {
  test.setTimeout(180_000);
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
    undefined,
    { timeout: 60_000 },
  );

  // Arm observers before reload so they catch the measured bursts.
  await page.addInitScript(() => {
    const store = window as unknown as {
      __INPUT_EVENTS__?: number[];
      __LONGTASKS__?: number[];
    };
    store.__INPUT_EVENTS__ = [];
    store.__LONGTASKS__ = [];
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.name === "input" || entry.name === "keydown") {
          store.__INPUT_EVENTS__?.push(entry.duration);
        }
      }
    }).observe({ type: "event", buffered: true, durationThreshold: 16 });
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        store.__LONGTASKS__?.push(entry.duration);
      }
    }).observe({ type: "longtask", buffered: true });
  });
  await page.reload();
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      Array.isArray(
        (window as unknown as { __INPUT_EVENTS__?: number[] }).__INPUT_EVENTS__,
      ),
    undefined,
    { timeout: 60_000 },
  );

  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents", {
    timeout: 30_000,
  });
  await expect(
    page.getByTestId("message-timeline").locator("[data-message-id]").first(),
  ).toBeVisible({ timeout: 30_000 });

  const client = await page.context().newCDPSession(page);
  await client.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE_RATE });

  await typeBurst(page);

  await resetWindowMetrics(page);
  await typeBurst(page);
  const quiet = await readWindowMetrics(page);

  await page.evaluate(
    ({ agentCount, typingIntervalMs, messageIntervalMs }) => {
      const w = window as unknown as {
        __BUZZ_E2E_EMIT_MOCK_TYPING__?: (input: {
          channelName: string;
          pubkey?: string;
        }) => unknown;
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
        }) => unknown;
        __BUSY_TIMERS__?: number[];
      };
      const pubkeys = Array.from({ length: agentCount }, (_, index) =>
        `a${index}`.repeat(32),
      );
      let tick = 0;
      const typingTimer = window.setInterval(() => {
        tick += 1;
        w.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
          channelName: "agents",
          pubkey: pubkeys[tick % pubkeys.length],
        });
      }, typingIntervalMs);
      let messageIndex = 0;
      const messageTimer = window.setInterval(() => {
        messageIndex += 1;
        w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "agents",
          content: `**Progress ${messageIndex}**\n\n- step done\n- \`cargo check\` ok`,
        });
      }, messageIntervalMs);
      w.__BUSY_TIMERS__ = [typingTimer, messageTimer];
    },
    {
      agentCount: AGENT_COUNT,
      typingIntervalMs: TYPING_EMIT_INTERVAL_MS,
      messageIntervalMs: LIVE_MESSAGE_INTERVAL_MS,
    },
  );
  await page.waitForTimeout(2000);

  await resetWindowMetrics(page);
  await typeBurst(page);
  const busy = await readWindowMetrics(page);

  await page.evaluate(() => {
    const w = window as unknown as { __BUSY_TIMERS__?: number[] };
    for (const timer of w.__BUSY_TIMERS__ ?? []) {
      window.clearInterval(timer);
    }
  });
  await client.send("Emulation.setCPUThrottlingRate", { rate: 1 });

  // eslint-disable-next-line no-console
  console.log(
    `WAVE1_PERF repeat=${testInfo.repeatEachIndex} quiet_median=${quiet.median.toFixed(0)} quiet_p95=${quiet.p95.toFixed(0)} quiet_over50=${quiet.over50} quiet_longtask=${quiet.longtaskTotal.toFixed(0)} busy_median=${busy.median.toFixed(0)} busy_p95=${busy.p95.toFixed(0)} busy_over50=${busy.over50} busy_longtask=${busy.longtaskTotal.toFixed(0)}`,
  );
});
