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

export type ActiveActionMeasurement = {
  before: BrowserMetrics;
  client: CDPSession;
  startedAt: number;
};

export async function beginActionMeasurement(
  page: Page,
): Promise<ActiveActionMeasurement> {
  const client = await page.context().newCDPSession(page);
  await client.send("Performance.enable");
  return {
    before: await readBrowserMetrics(client),
    client,
    startedAt: performance.now(),
  };
}

export async function finishActionMeasurement<T>(
  page: Page,
  measurement: ActiveActionMeasurement,
  result: T,
): Promise<ActionMeasurement<T>> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
      ),
  );
  return {
    metrics: delta(
      await readBrowserMetrics(measurement.client),
      measurement.before,
    ),
    result,
    wallMs: performance.now() - measurement.startedAt,
  };
}

export async function closeActionMeasurement(
  measurement: ActiveActionMeasurement,
): Promise<void> {
  await measurement.client.send("Performance.disable");
  await measurement.client.detach();
}

export async function measureAction<T>(
  page: Page,
  action: () => Promise<T>,
): Promise<ActionMeasurement<T>> {
  const measurement = await beginActionMeasurement(page);
  try {
    return await finishActionMeasurement(page, measurement, await action());
  } finally {
    await closeActionMeasurement(measurement);
  }
}
