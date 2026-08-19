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

test("agents-data-changed invalidates registered key alongside existing local library keys", () => {
  const localKeysBlock = refresh.slice(
    refresh.indexOf("LOCAL_AGENT_DATA_QUERY_KEYS"),
    refresh.indexOf("] as const"),
  );
  assert.match(localKeysBlock, /registeredAgentsQueryKey/);
  assert.match(localKeysBlock, /personasQueryKey/);
  assert.match(localKeysBlock, /teamsQueryKey/);
  assert.match(localKeysBlock, /managedAgentsQueryKey/);
  assert.doesNotMatch(localKeysBlock, /relayAgentsQueryKey/);

  const listenerBlock = refresh.slice(
    refresh.indexOf('listen("agents-data-changed"'),
    refresh.indexOf("return () =>"),
  );
  assert.match(listenerBlock, /LOCAL_AGENT_DATA_QUERY_KEYS/);
  assert.doesNotMatch(listenerBlock, /managedAgentRuntimesQueryKey/);
});
