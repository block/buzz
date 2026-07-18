import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * Composer typing-latency benchmark.
 *
 * Measures per-keystroke input-to-paint latency in the message composer,
 * quiet vs. under simulated agent activity (typing indicators from N agent
 * pubkeys + periodic live messages), to quantify the "typing feels slow in
 * agent-busy channels" report.
 *
 * METRIC: the browser's Event Timing API (`PerformanceObserver` type
 * "event") — each entry's `duration` is input timestamp → next paint, the
 * engine-level definition of keystroke responsiveness (8ms granularity).
 * We record entries for `input` events with durationThreshold 16ms and
 * report median/p95/max plus the count of >50ms (frame-budget-blowing)
 * keystrokes. Longtask totals are captured per scenario as a second axis.
 *
 * SCENARIOS (same 80-char typing burst, 4x CPU throttle):
 *   quiet — no agent traffic.
 *   busy  — 8 agent pubkeys emit typing indicators round-robin every 250ms
 *           (≈ the arrival rate of 8 agents refreshing every 2-3s), plus a
 *           live markdown message lands every 2s.
 *
 * Absolute ms are machine-specific; the quiet-vs-busy DELTA on one machine
 * is the signal. Run it (from desktop/):
 *   pnpm build
 *   npx playwright test --config=playwright.perf.config.ts typing-latency.perf.ts
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

async function typeBurst(
  page: import("@playwright/test").Page,
  scope?: import("@playwright/test").Locator,
) {
  const input = (scope ?? page).getByTestId("message-input").last();
  await input.click();
  await input.pressSequentially(TYPED_TEXT, { delay: KEY_DELAY_MS });
  // Let trailing event-timing entries (reported after paint) flush.
  await page.waitForTimeout(500);
  await input.press("Meta+A");
  await input.press("Backspace");
  await page.waitForTimeout(300);
}

function log(label: string, report: LatencyReport) {
  /* eslint-disable no-console */
  console.log(`\n=== TYPING LATENCY: ${label} ===`);
  console.log(`CPU throttle:            ${THROTTLE_RATE}x`);
  console.log(`input events >=16ms:     ${report.count}`);
  console.log(`median duration:         ${report.median.toFixed(0)}ms`);
  console.log(`p95 duration:            ${report.p95.toFixed(0)}ms`);
  console.log(`max duration:            ${report.max.toFixed(0)}ms`);
  console.log(`keystrokes >50ms:        ${report.over50}`);
  console.log(`longtask total:          ${report.longtaskTotal.toFixed(0)}ms`);
  /* eslint-enable no-console */
}

test("MEASURE: composer keystroke latency, quiet vs agent-busy channel", async ({
  page,
}) => {
  test.setTimeout(240_000);
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

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
  );

  // The `agents` channel: agent members, realistic surface for the report.
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");
  await expect(
    page.getByTestId("message-timeline").locator("[data-message-id]").first(),
  ).toBeVisible();

  const client = await page.context().newCDPSession(page);
  await client.send("Emulation.setCPUThrottlingRate", { rate: THROTTLE_RATE });

  // Warmup burst (JIT, first-render costs), unmeasured.
  await typeBurst(page);

  // ---- Scenario A: quiet ----
  await resetWindowMetrics(page);
  await typeBurst(page);
  const quiet = await readWindowMetrics(page);
  log("quiet channel", quiet);

  // ---- Scenario B: agent-busy ----
  // Round-robin typing indicators from N synthetic agent pubkeys, plus a
  // live markdown message every 2s — in-page timers so the traffic keeps
  // flowing while pressSequentially runs.
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
  // Let the busy traffic reach steady state (typing TTLs, working badges).
  await page.waitForTimeout(2000);

  await resetWindowMetrics(page);
  await typeBurst(page);
  const busy = await readWindowMetrics(page);
  log(`agent-busy (${AGENT_COUNT} agents typing + live messages)`, busy);

  await page.evaluate(() => {
    const w = window as unknown as { __BUSY_TIMERS__?: number[] };
    for (const timer of w.__BUSY_TIMERS__ ?? []) {
      window.clearInterval(timer);
    }
  });

  // ---- Scenario C: observer-frame storm ----
  // Pre-fill 6 agents' observer buffers (1200 events each), then append one
  // frame every 100ms round-robin. Each append runs the production ingestion
  // path: O(E log E) buffer re-sort + the syncAll bridge's O(agents x
  // events) rescan (see observerRelayStore.appendAgentEvent /
  // activeAgentTurnsStore syncAll) — the suspected typing-lag source.
  await page.evaluate(
    ({ agentCount, bufferSize, channelId }) => {
      const w = window as unknown as {
        __BUZZ_E2E_SEED_OBSERVER_EVENTS__?: (input: {
          agentPubkey: string;
          events: unknown[];
        }) => void;
        __OBS_AGENTS__?: Array<{ pubkey: string; seq: number }>;
      };
      const base = Date.parse("2025-06-15T12:00:00Z");
      const makeEvent = (pubkey: string, seq: number) => ({
        seq,
        timestamp: new Date(base + seq * 1000).toISOString(),
        kind: "acp_write",
        agentIndex: 0,
        channelId,
        sessionId: `sess-${pubkey.slice(0, 4)}`,
        turnId: `turn-${pubkey.slice(0, 4)}`,
        payload: {
          jsonrpc: "2.0",
          method: "session/update",
          params: { note: `event ${seq}` },
        },
      });
      w.__OBS_AGENTS__ = Array.from({ length: agentCount }, (_, index) => ({
        pubkey: `b${index}`.repeat(32),
        seq: bufferSize,
      }));
      for (const agent of w.__OBS_AGENTS__) {
        w.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
          agentPubkey: agent.pubkey,
          events: Array.from({ length: bufferSize }, (_, seq) =>
            makeEvent(agent.pubkey, seq),
          ),
        });
      }
      let stormTick = 0;
      const stormTimer = window.setInterval(() => {
        const agents = w.__OBS_AGENTS__ ?? [];
        if (agents.length === 0) return;
        stormTick += 1;
        const agent = agents[stormTick % agents.length];
        agent.seq += 1;
        w.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
          agentPubkey: agent.pubkey,
          events: [makeEvent(agent.pubkey, agent.seq)],
        });
      }, 100);
      (w as unknown as { __STORM_TIMER__?: number }).__STORM_TIMER__ =
        stormTimer;
    },
    {
      agentCount: 6,
      bufferSize: 1200,
      channelId: "94a444a4-c0a3-5966-ab05-530c6ddc2301", // #agents
    },
  );
  await page.waitForTimeout(1500);

  await resetWindowMetrics(page);
  await typeBurst(page);
  const storm = await readWindowMetrics(page);
  log("observer storm (6 agents, 1200-event buffers, 10 frames/s)", storm);

  // ---- Scenario D: everything at once (storm + typing + messages) ----
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
  await page.waitForTimeout(1000);

  await resetWindowMetrics(page);
  await typeBurst(page);
  const everything = await readWindowMetrics(page);
  log("storm + typing + messages", everything);

  await page.evaluate(() => {
    const w = window as unknown as {
      __BUSY_TIMERS__?: number[];
      __STORM_TIMER__?: number;
    };
    for (const timer of w.__BUSY_TIMERS__ ?? []) {
      window.clearInterval(timer);
    }
    if (w.__STORM_TIMER__) window.clearInterval(w.__STORM_TIMER__);
  });

  // ---- Scenario E: streaming agent into a full markdown timeline ----
  // The one O(rows) path is message-content change: each kind-40003 edit
  // re-runs formatTimelineMessages over the whole window and re-parses the
  // edited row's (growing) markdown. Agents streaming replies emit exactly
  // this shape at high frequency. Seed a realistic timeline first.
  await page.evaluate(
    ({ rows }) => {
      const w = window as unknown as {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          createdAt?: number;
        }) => { id: string };
      };
      const base = Math.floor(Date.now() / 1000) - rows - 30;
      for (let index = 0; index < rows; index += 1) {
        w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "agents",
          content: [
            `**Task ${index}** update from the build agent`,
            "```rust",
            `fn step_${index}() -> Result<(), Error> { run(${index}) }`,
            "```",
            `- [x] compile ${index}`,
            `- see [logs](https://example.com/${index})`,
          ].join("\n"),
          createdAt: base + index,
        });
      }
    },
    { rows: 50 },
  );
  await page.waitForTimeout(1500);

  await page.evaluate(
    ({ streamCount, prefillLines }) => {
      const w = window as unknown as {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          kind?: number;
          extraTags?: string[][];
        }) => { id: string };
        __STREAM_TIMERS__?: number[];
      };
      w.__STREAM_TIMERS__ = [];
      for (let stream = 0; stream < streamCount; stream += 1) {
        const target = w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "agents",
          content: "Working on it…",
        });
        if (!target) continue;
        // Pre-grown reply: mid-stream in a long code fence, the realistic
        // worst case — every edit re-parses the whole accumulated content.
        const chunks: string[] = ["**Streaming reply**", "", "```ts"];
        for (let line = 0; line < prefillLines; line += 1) {
          chunks.push(`const step${line} = await runStep(${line});`);
        }
        let line = prefillLines;
        const streamTimer = window.setInterval(() => {
          line += 1;
          chunks.push(`const step${line} = await runStep(${line}); // live`);
          w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
            channelName: "agents",
            content: `${chunks.join("\n")}\n\`\`\``,
            kind: 40003,
            extraTags: [["e", target.id]],
          });
        }, 120);
        w.__STREAM_TIMERS__?.push(streamTimer);
      }
    },
    { streamCount: 3, prefillLines: 150 },
  );
  await page.waitForTimeout(1000);

  await resetWindowMetrics(page);
  await typeBurst(page);
  const streaming = await readWindowMetrics(page);
  log(
    "3 streaming agents (150-line replies, edit every 120ms each)",
    streaming,
  );

  await page.evaluate(() => {
    const w = window as unknown as { __STREAM_TIMERS__?: number[] };
    for (const timer of w.__STREAM_TIMERS__ ?? []) {
      window.clearInterval(timer);
    }
  });

  // ---- Scenario F: typing in a long thread (the real-world repro) ----
  // Field report: a 68-reply thread with active agents is near-unusable to
  // type in (median 408ms, p95 8s+ per keystroke in WKWebView), while a
  // fresh thread in the same channel is instant — cost scales with the open
  // thread's reply count. Seed a 68-reply thread, open it, and type; then
  // repeat with agents streaming replies into the thread.
  const rootId = await page.evaluate(() => {
    const w = window as unknown as {
      __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
        channelName: string;
        content: string;
        parentEventId?: string;
        createdAt?: number;
      }) => { id: string };
    };
    const base = Math.floor(Date.now() / 1000) - 300;
    const root = w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "agents",
      content: "**Deploy thread** — agents report here",
      createdAt: base,
    });
    if (!root) return null;
    for (let index = 0; index < 68; index += 1) {
      w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "agents",
        content: [
          `**Reply ${index}** from agent`,
          "```rust",
          `fn check_${index}() -> bool { ${index} % 2 == 0 }`,
          "```",
          `- [x] validated step ${index}`,
        ].join("\n"),
        parentEventId: root.id,
        createdAt: base + index + 1,
      });
    }
    return root.id;
  });
  expect(rootId).not.toBeNull();
  await page.waitForTimeout(1500);

  const threadSummary = page.getByTestId("message-thread-summary").last();
  await threadSummary.scrollIntoViewIfNeeded();
  await threadSummary.click();
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-row").first()).toBeVisible();
  await page.waitForTimeout(1000);

  // F1: long thread, no traffic.
  await typeBurst(page, threadPanel); // warmup
  await resetWindowMetrics(page);
  await typeBurst(page, threadPanel);
  const threadQuiet = await readWindowMetrics(page);
  log("68-reply thread, quiet", threadQuiet);

  // F2: agents streaming into the open thread while typing.
  await page.evaluate(
    ({ rootEventId }) => {
      const w = window as unknown as {
        __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
          channelName: string;
          content: string;
          parentEventId?: string;
          kind?: number;
          extraTags?: string[][];
        }) => { id: string };
        __THREAD_TIMERS__?: number[];
      };
      // A new agent reply lands every 600ms…
      let replyIndex = 0;
      let lastReplyId: string | null = null;
      const replyTimer = window.setInterval(() => {
        replyIndex += 1;
        const reply = w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "agents",
          content: `**Live reply ${replyIndex}**\n\n- working…`,
          parentEventId: rootEventId ?? undefined,
        });
        lastReplyId = reply?.id ?? lastReplyId;
      }, 600);
      // …and the latest reply streams edits every 150ms.
      const chunks: string[] = ["**Live reply**", "", "```ts"];
      let line = 0;
      const editTimer = window.setInterval(() => {
        if (!lastReplyId) return;
        line += 1;
        chunks.push(`const s${line} = await step(${line});`);
        w.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "agents",
          content: `${chunks.join("\n")}\n\`\`\``,
          kind: 40003,
          extraTags: [["e", lastReplyId]],
        });
      }, 150);
      w.__THREAD_TIMERS__ = [replyTimer, editTimer];
    },
    { rootEventId: rootId },
  );
  await page.waitForTimeout(1000);

  await resetWindowMetrics(page);
  await typeBurst(page, threadPanel);
  const threadBusy = await readWindowMetrics(page);
  log("68-reply thread + agents streaming into it", threadBusy);

  await page.evaluate(() => {
    const w = window as unknown as { __THREAD_TIMERS__?: number[] };
    for (const timer of w.__THREAD_TIMERS__ ?? []) {
      window.clearInterval(timer);
    }
  });
  await client.send("Emulation.setCPUThrottlingRate", { rate: 1 });

  /* eslint-disable no-console */
  console.log("\n=== SUMMARY (median / p95 / >50ms count / longtask) ===");
  for (const [label, report] of [
    ["quiet     ", quiet],
    ["busy      ", busy],
    ["storm     ", storm],
    ["everything", everything],
    ["streaming ", streaming],
    ["thread68  ", threadQuiet],
    ["thread68+ ", threadBusy],
  ] as const) {
    console.log(
      `${label}: ${report.median.toFixed(0)} / ${report.p95.toFixed(0)} / ${report.over50} / ${report.longtaskTotal.toFixed(0)}ms`,
    );
  }
  /* eslint-enable no-console */

  // Instrument, not a gate: confirm the harness measured real keystrokes.
  expect(quiet.count + busy.count + storm.count).toBeGreaterThan(0);
});
