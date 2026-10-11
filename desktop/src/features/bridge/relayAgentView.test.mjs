import assert from "node:assert/strict";
import test from "node:test";
import { hiddenAgentCount, NO_RUNNING_AGENTS, visibleRelayAgents } from "./relayAgentView.ts";

const a = (status) => ({ status });
test("relay bridge lists only running agents", () => {
  const all = [a("running"), a("stopped"), a("deployed"), a("not_deployed"), a("running")];
  assert.equal(visibleRelayAgents(all).length, 2);
  assert.ok(visibleRelayAgents(all).every((x) => x.status === "running"));
  assert.equal(hiddenAgentCount(all), 3);
});
test("none running -> empty list and the empty-state copy", () => {
  assert.deepEqual(visibleRelayAgents([a("stopped")]), []);
  assert.equal(NO_RUNNING_AGENTS, "No agents running on your desktop.");
});
test("direct connection (showAll) keeps everything", () => {
  const all = [a("running"), a("stopped")];
  assert.equal(visibleRelayAgents(all, true).length, 2);
  assert.equal(hiddenAgentCount(all, true), 0);
});
