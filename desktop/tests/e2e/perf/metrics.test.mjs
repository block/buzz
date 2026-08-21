import assert from "node:assert/strict";
import test from "node:test";

const { measureAction } = await import("./metrics.ts");

test("measureAction keeps action between baseline metrics and settled observation", async () => {
  const operations = [];
  let metricsReads = 0;
  const client = {
    async send(method) {
      operations.push(method);
      if (method === "Performance.getMetrics") {
        metricsReads += 1;
        return {
          metrics: [
            { name: "LayoutDuration", value: metricsReads },
            { name: "RecalcStyleDuration", value: 0 },
            { name: "LayoutCount", value: 0 },
            { name: "ScriptDuration", value: 0 },
            { name: "TaskDuration", value: 0 },
          ],
        };
      }
      return {};
    },
    async detach() {
      operations.push("detach");
    },
  };
  const page = {
    context: () => ({ newCDPSession: async () => client }),
    async evaluate() {
      operations.push("settle-render");
    },
  };

  const measurement = await measureAction(page, async () => {
    operations.push("bootstrap-navigation-first-render");
    return "result";
  });

  assert.equal(measurement.result, "result");
  assert.equal(measurement.metrics.layoutMs, 1000);
  assert.deepEqual(operations, [
    "Performance.enable",
    "Performance.getMetrics",
    "bootstrap-navigation-first-render",
    "settle-render",
    "Performance.getMetrics",
    "Performance.disable",
    "detach",
  ]);
});
