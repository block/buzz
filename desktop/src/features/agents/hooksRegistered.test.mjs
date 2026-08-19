import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const hooks = readFileSync(new URL("./hooks.ts", import.meta.url), "utf8");
const refresh = readFileSync(
  new URL("./lib/useAgentsDataRefresh.ts", import.meta.url),
  "utf8",
);

test("registered mutations invalidate only the registered-agent query key", () => {
  assert.match(hooks, /registeredAgentsQueryKey/);
  assert.match(hooks, /useRegisterExistingAgentMutation/);
  assert.match(hooks, /useUnregisterExistingAgentMutation/);
  const registeredBlock = hooks.slice(
    hooks.indexOf("useRegisterExistingAgentMutation"),
    hooks.indexOf("useUnregisterExistingAgentMutation"),
  );
  assert.match(registeredBlock, /queryKey: registeredAgentsQueryKey/);
  assert.doesNotMatch(
    registeredBlock,
    /managedAgentsQueryKey|relayAgentsQueryKey|managedAgentRuntimesQueryKey/,
  );
});

test("agents-data-changed invalidates registered key alongside existing library keys", () => {
  const block = refresh.slice(
    refresh.indexOf('listen("agents-data-changed"'),
    refresh.indexOf("return () =>"),
  );
  assert.match(block, /registeredAgentsQueryKey/);
  assert.match(block, /personasQueryKey/);
  assert.match(block, /teamsQueryKey/);
  assert.match(block, /managedAgentsQueryKey/);
  assert.match(block, /relayAgentsQueryKey/);
  assert.doesNotMatch(block, /managedAgentRuntimesQueryKey/);
});
