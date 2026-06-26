import type { CDPSession, Page } from "@playwright/test";

export type BrowserMetrics = {
  layoutMs: number;
  recalcMs: number;
  layoutCount: number;
  scriptMs: number;
  taskMs: number;
};

export type ActionMeasurement<T> = {
  metrics: BrowserMetrics;
  result: T;
  wallMs: number;
};

export async function readBrowserMetrics(
  client: CDPSession,
): Promise<BrowserMetrics> {
  const { metrics } = (await client.send("Performance.getMetrics")) as {
    metrics: Array<{ name: string; value: number }>;
  };
  const m = (name: string) => metrics.find((x) => x.name === name)?.value ?? 0;
  return {
    // CDP reports durations in seconds; convert to ms.
    layoutMs: m("LayoutDuration") * 1000,
    recalcMs: m("RecalcStyleDuration") * 1000,
    layoutCount: m("LayoutCount"),
    scriptMs: m("ScriptDuration") * 1000,
    taskMs: m("TaskDuration") * 1000,
  };
}

function deltaMetrics(after: BrowserMetrics, before: BrowserMetrics) {
  return {
    layoutMs: after.layoutMs - before.layoutMs,
    recalcMs: after.recalcMs - before.recalcMs,
    layoutCount: after.layoutCount - before.layoutCount,
    scriptMs: after.scriptMs - before.scriptMs,
    taskMs: after.taskMs - before.taskMs,
  } satisfies BrowserMetrics;
}

export async function measureAction<T>(
  page: Page,
  action: () => Promise<T>,
): Promise<ActionMeasurement<T>> {
  const client = await page.context().newCDPSession(page);
  await client.send("Performance.enable");
  const before = await readBrowserMetrics(client);
  const start = performance.now();
  const result = await action();
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve())),
  );
  const wallMs = performance.now() - start;
  const after = await readBrowserMetrics(client);
  await client.send("Performance.disable");
  return { metrics: deltaMetrics(after, before), result, wallMs };
}

export function logMeasurement(
  title: string,
  fields: Record<string, string | number>,
) {
  const width = Math.max(...Object.keys(fields).map((key) => key.length));
  /* eslint-disable no-console */
  console.log(`\n=== ${title} ===`);
  for (const [key, value] of Object.entries(fields)) {
    console.log(`${key.padEnd(width)}: ${value}`);
  }
  console.log("========================================\n");
  /* eslint-enable no-console */
}
