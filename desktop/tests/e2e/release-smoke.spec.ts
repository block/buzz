import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

import { expect, test, type Page, type TestInfo } from "@playwright/test";

import { installRelayBridge } from "../helpers/bridge";
import { assertRelaySeeded } from "../helpers/seed";
import { denseSecondWall, seedScenario } from "../helpers/seedRelay";
import { measureAction, type ActionMeasurement } from "./perf/metrics";

const RELAY_HTTP = process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";
const GENERAL_CHANNEL_ID = "9f28288a-d724-587a-9709-92dc7f967110";
const DEEP_ROW_COUNT = Number.parseInt(
  process.env.BUZZ_RELEASE_SMOKE_ROWS ?? "10000",
  10,
);
const FIXTURE_VERSION = 1;

type ScenarioArtifact = {
  schemaVersion: 1;
  fixtureVersion: number;
  fixtureSecond: number;
  scenario: string;
  relayUrl: string;
  rowCount: number;
  expectedIdHash: string;
  observed: Record<string, unknown>;
  measurement: Omit<ActionMeasurement<unknown>, "result">;
};

async function sha256(values: readonly string[]) {
  const bytes = new TextEncoder().encode([...values].sort().join("\n"));
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

async function writeArtifact(testInfo: TestInfo, artifact: ScenarioArtifact) {
  const body = `${JSON.stringify(artifact, null, 2)}\n`;
  const outputPath = testInfo.outputPath("metrics.json");
  await writeFile(outputPath, body);
  await testInfo.attach("release-smoke-metrics", {
    body: Buffer.from(body),
    contentType: "application/json",
  });

  const requestedDirectory = process.env.BUZZ_RELEASE_SMOKE_ARTIFACT_DIR;
  if (requestedDirectory) {
    const directory = resolve(requestedDirectory);
    await mkdir(directory, { recursive: true });
    await writeFile(resolve(directory, `${artifact.scenario}.json`), body);
  }
}

async function openGeneral(page: Page) {
  await installRelayBridge(page, "tyler");
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.locator('[data-render-pending="true"]')).toHaveCount(0);
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();
  return timeline;
}

async function mountedRowCount(page: Page) {
  return page.getByTestId("message-row").count();
}

test.beforeAll(async () => {
  test.setTimeout(90_000);
  await assertRelaySeeded();
});

test("deep local-relay timeline remains bounded and every row is reachable", async ({
  page,
}, testInfo) => {
  testInfo.setTimeout(10 * 60_000);

  const fixtureSecond = Math.floor(Date.now() / 1000) - 1;
  const fixture = denseSecondWall({
    channelId: GENERAL_CHANNEL_ID,
    count: DEEP_ROW_COUNT,
    second: fixtureSecond,
  });
  const expected = await seedScenario(fixture, {
    relayHttpUrl: RELAY_HTTP,
    concurrency: 64,
  });
  expect(expected).toHaveLength(DEEP_ROW_COUNT);
  const expectedIdHash = await sha256(expected.map(({ id }) => id));
  const expectedOrder = expected.map(({ id }) => id).sort();
  const expectedRank = new Map(expectedOrder.map((id, index) => [id, index]));

  const measurement = await measureAction(page, async () => {
    const timeline = await openGeneral(page);
    const initialMountedRows = await mountedRowCount(page);
    let duplicateSnapshots = 0;
    let orderingViolations = 0;
    let orderDirection: "ascending" | "descending" | null = null;
    let maxMountedRows = initialMountedRows;
    let renderPendingTimeouts = 0;
    const seen = new Set<string>();
    const collect = async () => {
      const ids = await timeline.evaluate((element) =>
        Array.from(element.querySelectorAll<HTMLElement>("[data-message-id]"))
          .map((row) => row.dataset.messageId)
          .filter((id): id is string => Boolean(id)),
      );
      if (new Set(ids).size !== ids.length) duplicateSnapshots += 1;
      const ranks = ids.map((id) => expectedRank.get(id) ?? -1);
      if (ranks.includes(-1)) orderingViolations += 1;
      for (let index = 1; index < ranks.length; index += 1) {
        if (ranks[index] === ranks[index - 1]) {
          orderingViolations += 1;
          continue;
        }
        const direction =
          ranks[index] > ranks[index - 1] ? "ascending" : "descending";
        orderDirection ??= direction;
        if (direction !== orderDirection) orderingViolations += 1;
      }
      for (const id of ids) seen.add(id);
    };

    await collect();
    await timeline.hover();
    const passes: Array<{
      pass: number;
      before: number;
      after: number;
      continuationRequests: number;
      scrollTop: number;
      scrollHeight: number;
    }> = [];
    let stalled = 0;
    let stallReason = "row-target-reached";
    const maxPasses = Math.ceil(DEEP_ROW_COUNT / 25) + 32;
    for (
      let pass = 0;
      pass < maxPasses && seen.size < DEEP_ROW_COUNT;
      pass += 1
    ) {
      const before = seen.size;
      // Keep wheel increments below a typical virtualized page height. Large
      // jumps can legitimately skip transiently mounted rows and turn a DOM
      // reachability assertion into a collector bug.
      for (let step = 0; step < 128; step += 1) {
        await page.mouse.wheel(0, -400);
        await page.waitForTimeout(20);
        await collect();
        const atTop = await timeline.evaluate(
          (element) => (element as HTMLDivElement).scrollTop <= 1,
        );
        if (atTop) break;
      }
      await expect
        .poll(
          async () => {
            await collect();
            return seen.size;
          },
          { timeout: 4_000 },
        )
        .toBeGreaterThan(before)
        .catch(() => {});
      try {
        await expect(page.locator('[data-render-pending="true"]')).toHaveCount(
          0,
          { timeout: 5_000 },
        );
      } catch {
        renderPendingTimeouts += 1;
      }
      await collect();
      const mounted = await mountedRowCount(page);
      maxMountedRows = Math.max(maxMountedRows, mounted);
      const state = await timeline.evaluate((element) => ({
        continuationRequests:
          (
            window as typeof window & {
              __CHANNEL_WINDOW_FETCH_COUNT__?: number;
            }
          ).__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0,
        scrollTop: (element as HTMLDivElement).scrollTop,
        scrollHeight: (element as HTMLDivElement).scrollHeight,
      }));
      passes.push({ pass, before, after: seen.size, ...state });
      stalled = seen.size === before ? stalled + 1 : 0;
      if (stalled >= 8) {
        stallReason = "eight-passes-without-new-rows";
        break;
      }
      stallReason = "pass-limit-reached";
    }

    if (seen.size >= DEEP_ROW_COUNT) stallReason = "row-target-reached";

    return {
      initialMountedRows,
      finalMountedRows: await mountedRowCount(page),
      maxMountedRows,
      duplicateSnapshots,
      orderingViolations,
      orderDirection,
      renderPendingTimeouts,
      reachableRows: seen.size,
      reachableIdHash: await sha256([...seen]),
      continuationRequests: await page.evaluate(
        () =>
          (
            window as typeof window & {
              __CHANNEL_WINDOW_FETCH_COUNT__?: number;
            }
          ).__CHANNEL_WINDOW_FETCH_COUNT__ ?? 0,
      ),
      passCount: passes.length,
      stallReason,
      passes,
    };
  });

  await writeArtifact(testInfo, {
    schemaVersion: 1,
    fixtureVersion: FIXTURE_VERSION,
    fixtureSecond,
    scenario: "deep-local-relay-timeline",
    relayUrl: RELAY_HTTP,
    rowCount: DEEP_ROW_COUNT,
    expectedIdHash,
    observed: measurement.result,
    measurement: {
      wallMs: measurement.wallMs,
      metrics: measurement.metrics,
    },
  });

  expect(measurement.result.initialMountedRows).toBeGreaterThan(0);
  expect(measurement.result.maxMountedRows).toBeLessThanOrEqual(300);
  expect(measurement.result.duplicateSnapshots).toBe(0);
  expect(measurement.result.orderingViolations).toBe(0);
  expect(measurement.result.renderPendingTimeouts).toBe(0);
  expect(measurement.result.reachableRows).toBe(DEEP_ROW_COUNT);
  expect(measurement.result.reachableIdHash).toBe(expectedIdHash);
});
