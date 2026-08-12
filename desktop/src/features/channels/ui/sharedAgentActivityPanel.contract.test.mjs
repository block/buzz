import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const panelSource = readFileSync(
  new URL("./AgentSessionThreadPanel.tsx", import.meta.url),
  "utf8",
);
const sharedPanelSource = readFileSync(
  new URL("../../agents/SharedAgentActivityPanel.tsx", import.meta.url),
  "utf8",
);

test("non-owners branch to the member-safe panel before owner telemetry hooks mount", () => {
  const wrapper = panelSource.indexOf(
    "export function AgentSessionThreadPanel",
  );
  const ownerPanel = panelSource.indexOf(
    "function OwnerAgentSessionThreadPanel",
  );
  const ownerHook = panelSource.indexOf("useObserverEvents(");
  assert.ok(wrapper >= 0, "public activity panel wrapper is present");
  assert.ok(
    ownerPanel > wrapper,
    "owner-only component is nested behind the wrapper",
  );
  assert.ok(
    ownerHook > ownerPanel,
    "raw observer hook mounts only inside owner component",
  );
  assert.match(
    panelSource.slice(wrapper, ownerPanel),
    /resolveAgentActivityMode/,
  );
  assert.match(
    panelSource.slice(wrapper, ownerPanel),
    /SharedAgentActivityPanel/,
  );
});

test("member-safe panel cannot import owner observer, archive, control, or raw-feed APIs", () => {
  assert.doesNotMatch(
    sharedPanelSource,
    /useObserverEvents|useArchivedChannelEvents|useLoadArchivedObserverEvents|cancelManagedAgentTurn|RawEvent|agentControl/,
  );
  assert.match(sharedPanelSource, /useSharedAgentActivity/);
});
