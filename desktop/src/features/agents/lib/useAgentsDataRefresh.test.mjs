import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const source = await readFile(
  new URL("./useAgentsDataRefresh.ts", import.meta.url),
  "utf8",
);
const collapsedSource = source.replace(/\s+/g, " ");

test("relay agent directory updates refresh mention eligibility live", () => {
  assert.match(
    collapsedSource,
    /subscribeLive\( \{ kinds: \[KIND_PROFILE, KIND_AGENT_PROFILE, KIND_MANAGED_AGENT\], limit: 0,/,
  );
  assert.match(
    collapsedSource,
    /invalidateQueries\(\{ queryKey: relayAgentsQueryKey \}\)/,
  );
});

test("relay reconnects refresh agent profiles missed while offline", () => {
  assert.match(
    collapsedSource,
    /subscribeToReconnects\(\(\) => \{ refreshRelayAgents\(\); \}\)/,
  );
});
