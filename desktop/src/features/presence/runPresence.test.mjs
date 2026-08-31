import assert from "node:assert/strict";
import test from "node:test";
import { activeRuns, locationLabels, nextRunExpiry } from "./runPresence.ts";
const run = (id, label, expires_at = 280) => ({
  run: id,
  seq: 1,
  status: "online",
  expires_at,
  location: label ? { host: id, label } : null,
  registration: null,
});
test("location requires its own unexpired run, never aggregate presence", () => {
  const runs = [run("a", "Workshop"), run("b", "Office", 300), run("c", null)];
  assert.deepEqual(locationLabels(runs, 100), ["Office", "Workshop"]);
  assert.deepEqual(locationLabels(runs, 280), ["Office"]);
  assert.deepEqual(locationLabels(runs, 300), []);
  assert.equal(nextRunExpiry({ agent: runs }, 280), 300);
  assert.equal(nextRunExpiry({ agent: runs }, 300), undefined);
});
test("stopping one placement leaves the other; repeated labels coalesce", () => {
  const first = { ...run("a", "Workshop"), status: "offline" };
  const runs = [first, run("b", "Office"), run("c", "Office")];
  assert.equal(activeRuns(runs, 100).length, 2);
  assert.deepEqual(locationLabels(runs, 100), ["Office"]);
  assert.deepEqual(locationLabels(undefined, 100), []);
});
