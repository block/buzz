import assert from "node:assert/strict";
import test from "node:test";
import { EventBudget } from "../src/event-budget.mjs";

function turn(usage, nested = []) {
  return {
    message: { usage },
    toolResults: nested.map((item) => ({ usage: item })),
  };
}

test("allows exactly the configured tool calls then blocks all later tools", () => {
  const budget = new EventBudget({ turns: 6, tools: 2, tokens: 200_000 });
  assert.equal(budget.onToolCall(), undefined);
  assert.equal(budget.onToolCall(), undefined);
  assert.equal(budget.snapshot().tools, 2);
  assert.equal(budget.onToolCall().block, true);
  assert.equal(budget.onToolCall().block, true);
  assert.equal(budget.snapshot().tools, 2);
});

test("counts fresh, cache, output, and nested tool usage once at turn end", () => {
  const budget = new EventBudget({ turns: 6, tools: 8, tokens: 1_000 });
  const outcome = budget.onTurnEnd(
    turn({ input: 100, output: 20, cacheRead: 80, cacheWrite: 5 }, [
      { input: 10, output: 2, cacheRead: 3, cacheWrite: 1 },
    ]),
  );
  assert.equal(outcome.checkpoint, false);
  assert.equal(budget.snapshot().tokens, 221);
  assert.equal(budget.snapshot().turns, 1);
});

test("threshold queues one checkpoint turn then forces abort", () => {
  const budget = new EventBudget({ turns: 1, tools: 8, tokens: 1_000 });
  const outcome = budget.onTurnEnd(turn({ input: 10, output: 1 }));
  assert.equal(outcome.checkpoint, true);
  assert.match(outcome.message, /turns 1\/1/);
  assert.equal(budget.onTurnStart(), "checkpoint");
  assert.equal(budget.onTurnStart(), "abort");
  assert.equal(budget.onTurnStart(), "abort");
  assert.equal(budget.snapshot().forcedAbort, true);
});

test("token threshold is inclusive", () => {
  const budget = new EventBudget({ turns: 6, tools: 8, tokens: 100 });
  assert.equal(
    budget.onTurnEnd(turn({ input: 50, output: 10, cacheRead: 40 })).checkpoint,
    true,
  );
});

test("reset creates an independent budget for the next inbound event", () => {
  const budget = new EventBudget({ turns: 1, tools: 1, tokens: 10 });
  budget.onToolCall();
  budget.onTurnEnd(turn({ input: 10 }));
  budget.onTurnStart();
  budget.reset();
  assert.deepEqual(budget.snapshot(), {
    turns: 0,
    tools: 0,
    tokens: 0,
    thresholdReached: false,
    checkpointTurnStarted: false,
    forcedAbort: false,
    limits: { turns: 1, tools: 1, tokens: 10 },
  });
  assert.equal(budget.onToolCall(), undefined);
});
