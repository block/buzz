import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * High-membership channel-switch benchmark.
 *
 * Isolates how channel MEMBERSHIP SIZE scales the warm-switch cost, holding
 * message volume constant. Every channel object embeds its full
 * member-pubkey array, so membership size inflates (a) the get_channels
 * payload parsed on every poll, (b) the per-switch get_channel_members
 * response, and (c) every render-path pass over `channel.memberPubkeys` and
 * the member list (profile merges, agent-flag merges, mention candidates).
 * This spec is the instrument for that scaling: same channels, same rows,
 * member count is the only variable.
 *
 * Two scenarios per member count:
 *   channel<->channel   — general <-> deep-history (150 fixed rows).
 *   channel<->projects  — general <-> the Projects overview (preview
 *                         feature). Projects mounts its own query fan
 *                         (project enumeration, work items, repo snapshots,
 *                         activity summaries) on top of the shell, so this
 *                         axis captures the cross-surface switch the felt
 *                         1-2s report singled out.
 *
 * Method mirrors warm-switch-markdown.perf.ts: in-page click + rAF polling
 * (CDP latency never pollutes samples), longtask capture per switch, 4x CPU
 * throttle, medians over repeated switches, untimed warmup round-trip first.
 * `deep-history` is pinned to 150 rows so the message-mount cost is fixed
 * and comparable across member counts.
 *
 * Run it (from desktop/):
 *   pnpm build:e2e
 *   npx playwright test --config=playwright.perf.config.ts member-heavy-switch.perf.ts
 *
 * Compare the MEDIAN wall ms / longtask lines across the member-count
 * scenarios; a superlinear jump is membership-scaling cost on the switch
 * path.
 */

const MEASURED_SWITCHES = 8;
const THROTTLE_RATE = 4;
const DEEP_HISTORY_ROWS = 150;
const MEMBER_COUNTS = [0, 2_000, 10_000] as const;

type SwitchSample = {
  ms: number;
  longtaskTotal: number;
  longtaskMax: number;
  longtaskCount: number;
};

function median(values: number[]): number {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[mid - 1] + sorted[mid]) / 2
    : sorted[mid];
}

/** Click the sidebar link and poll — all in-page — until the target channel's
 * rows are committed, the deferred snapshot has caught up, and a frame
 * painted. Returns wall-clock ms plus the longtasks observed in the window. */
async function measureSwitch(
  page: import("@playwright/test").Page,
  input: {
    targetTestId: string;
    /** When set, chat-title must equal this before the switch counts. */
    targetTitle: string | null;
    /** Selector that must be present before the switch counts. */
    readySelector: string;
  },
): Promise<SwitchSample> {
  return page.evaluate(async (args) => {
    const store = window as unknown as { __LONGTASKS__: number[] };
    store.__LONGTASKS__ = [];
    const link = document.querySelector<HTMLElement>(
      `[data-testid="${args.targetTestId}"]`,
    );
    if (!link) throw new Error(`missing sidebar link ${args.targetTestId}`);

    const start = performance.now();
    link.click();

    await new Promise<void>((resolve, reject) => {
      const deadline = start + 30_000;
      const check = () => {
        const titleReady =
          args.targetTitle === null ||
          document.querySelector('[data-testid="chat-title"]')?.textContent ===
            args.targetTitle;
        const ready =
          titleReady &&
          document.querySelector(args.readySelector) !== null &&
          document.querySelector('[data-render-pending="true"]') === null;
        if (ready) {
          requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
          return;
        }
        if (performance.now() > deadline) {
          reject(new Error(`switch to ${args.targetTitle} timed out`));
          return;
        }
        requestAnimationFrame(check);
      };
      requestAnimationFrame(check);
    });

    const elapsed = performance.now() - start;
    const tasks = store.__LONGTASKS__ ?? [];
    return {
      ms: elapsed,
      longtaskTotal: tasks.reduce((sum, duration) => sum + duration, 0),
      longtaskMax: tasks.length ? Math.max(...tasks) : 0,
      longtaskCount: tasks.length,
    };
  }, input);
}

type SwitchTarget = {
  targetTestId: string;
  targetTitle: string | null;
  readySelector: string;
};

const GENERAL_TARGET: SwitchTarget = {
  targetTestId: "channel-general",
  targetTitle: "general",
  readySelector: "[data-message-id]",
};

const DEEP_HISTORY_TARGET: SwitchTarget = {
  targetTestId: "channel-deep-history",
  targetTitle: "deep-history",
  readySelector: '[data-message-id^="mock-deep-history-"]',
};

const PROJECTS_TARGET: SwitchTarget = {
  targetTestId: "open-projects-view",
  targetTitle: null,
  // Rendered by every Projects view mode (Activity intro or section header).
  readySelector: '[data-testid="projects-page-header"]',
};

async function runScenario(
  page: import("@playwright/test").Page,
  label: string,
  target: SwitchTarget,
  back: SwitchTarget,
): Promise<SwitchSample[]> {
  // Untimed warmup round-trip: caches both surfaces' queries and jits the
  // switch code paths.
  await measureSwitch(page, target);
  await measureSwitch(page, back);

  const samples: SwitchSample[] = [];
  for (let run = 0; run < MEASURED_SWITCHES; run += 1) {
    samples.push(await measureSwitch(page, target));
    samples.push(await measureSwitch(page, back));
  }

  const times = samples.map((sample) => sample.ms);
  const longtaskTotals = samples.map((sample) => sample.longtaskTotal);
  /* eslint-disable no-console */
  console.log(`\n=== MEMBER-HEAVY WARM SWITCH: ${label} ===`);
  console.log(`CPU throttle:            ${THROTTLE_RATE}x`);
  console.log(
    `per-switch wall ms:      [${times.map((v) => v.toFixed(1)).join(", ")}]`,
  );
  console.log(
    `per-switch longtask ms:  [${longtaskTotals.map((v) => v.toFixed(1)).join(", ")}]`,
  );
  console.log(`MEDIAN wall ms:          ${median(times).toFixed(1)}`);
  console.log(
    `MEDIAN longtask total:   ${median(longtaskTotals).toFixed(1)}ms`,
  );
  console.log(
    `worst single longtask:   ${Math.max(...samples.map((sample) => sample.longtaskMax)).toFixed(1)}ms`,
  );
  /* eslint-enable no-console */
  return samples;
}

for (const memberCount of MEMBER_COUNTS) {
  const label =
    memberCount === 0
      ? "baseline fixture membership"
      : `${memberCount.toLocaleString("en-US")} members per channel`;

  test(`MEASURE: warm switch general<->deep-history and general<->projects with ${label}`, async ({
    page,
  }) => {
    test.setTimeout(300_000);
    // Projects is a preview feature; seed the override BEFORE the bridge
    // installs so the shell mounts with it enabled.
    await page.addInitScript(() => {
      window.localStorage.setItem(
        "buzz-feature-overrides-v1",
        JSON.stringify({ projects: true }),
      );
    });
    await installMockBridge(page, {
      deepHistoryMessageCount: DEEP_HISTORY_ROWS,
      ...(memberCount > 0
        ? {
            inflateChannelMembers: {
              general: memberCount,
              "deep-history": memberCount,
            },
          }
        : {}),
    });
    await page.goto("/");
    await page.waitForFunction(
      () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
    );

    // Arm the longtask observer; addInitScript applies on next navigation.
    await page.addInitScript(() => {
      const store = window as unknown as { __LONGTASKS__?: number[] };
      store.__LONGTASKS__ = [];
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
          (window as unknown as { __LONGTASKS__?: number[] }).__LONGTASKS__,
        ),
    );

    // Verify the inflation actually landed before measuring anything.
    if (memberCount > 0) {
      await expect
        .poll(() =>
          page.evaluate(async () => {
            const invoke = (
              window as unknown as {
                __TAURI_INTERNALS__: {
                  invoke: (
                    cmd: string,
                    args: unknown,
                  ) => Promise<{ members: unknown[] }>;
                };
              }
            ).__TAURI_INTERNALS__.invoke;
            const response = await invoke("get_channel_members", {
              channelId: "feedf00d-0000-4000-8000-000000000007",
            });
            return response.members.length;
          }),
        )
        .toBeGreaterThanOrEqual(memberCount);
    }

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    const client = await page.context().newCDPSession(page);
    await client.send("Emulation.setCPUThrottlingRate", {
      rate: THROTTLE_RATE,
    });

    const channelSamples = await runScenario(
      page,
      `channel<->channel, ${label}`,
      DEEP_HISTORY_TARGET,
      GENERAL_TARGET,
    );
    const projectsSamples = await runScenario(
      page,
      `channel<->projects, ${label}`,
      PROJECTS_TARGET,
      GENERAL_TARGET,
    );

    await client.send("Emulation.setCPUThrottlingRate", { rate: 1 });

    // Instrument, not a gate: assert the harness measured real work.
    expect(channelSamples.length).toBe(MEASURED_SWITCHES * 2);
    expect(channelSamples.every((sample) => sample.ms > 0)).toBe(true);
    expect(projectsSamples.length).toBe(MEASURED_SWITCHES * 2);
    expect(projectsSamples.every((sample) => sample.ms > 0)).toBe(true);
  });
}
