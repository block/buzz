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

async function readBrowserMetrics(client: CDPSession): Promise<BrowserMetrics> {
  const { metrics } = (await client.send("Performance.getMetrics")) as {
    metrics: Array<{ name: string; value: number }>;
  };
  const metric = (name: string) =>
    metrics.find((candidate) => candidate.name === name)?.value ?? 0;
  return {
    layoutMs: metric("LayoutDuration") * 1000,
    recalcMs: metric("RecalcStyleDuration") * 1000,
    layoutCount: metric("LayoutCount"),
    scriptMs: metric("ScriptDuration") * 1000,
    taskMs: metric("TaskDuration") * 1000,
  };
}

function delta(after: BrowserMetrics, before: BrowserMetrics): BrowserMetrics {
  return {
    layoutMs: after.layoutMs - before.layoutMs,
    recalcMs: after.recalcMs - before.recalcMs,
    layoutCount: after.layoutCount - before.layoutCount,
    scriptMs: after.scriptMs - before.scriptMs,
    taskMs: after.taskMs - before.taskMs,
  };
}

export async function measureAction<T>(
  page: Page,
  action: () => Promise<T>,
): Promise<ActionMeasurement<T>> {
  const client = await page.context().newCDPSession(page);
  await client.send("Performance.enable");
  try {
    const before = await readBrowserMetrics(client);
    const startedAt = performance.now();
    const result = await action();
    await page.evaluate(
      () =>
        new Promise<void>((resolve) =>
          requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
        ),
    );
    const wallMs = performance.now() - startedAt;
    return {
      metrics: delta(await readBrowserMetrics(client), before),
      result,
      wallMs,
    };
  } finally {
    await client.send("Performance.disable");
    await client.detach();
  }
}
