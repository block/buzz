import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const AGENT =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SCOPE = "owner:channel:channel";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  const { resetPersistentAgentAudienceStore } = await import(
    "@/features/messages/lib/persistentAgentAudience.ts"
  );
  resetPersistentAgentAudienceStore();
});

after(() => dom.window.close());

async function renderAutoPin({ enabled }) {
  const { act, renderHook } = await import("@testing-library/react");
  const { useAutoPinMentionedAgents } = await import(
    "./useAutoPinMentionedAgents.ts"
  );
  const { getPersistentAgentAudienceSnapshot } = await import(
    "@/features/messages/lib/persistentAgentAudience.ts"
  );

  const turnOnCalls = [];
  const pulses = [];
  const { result } = renderHook(() =>
    useAutoPinMentionedAgents({
      audienceScope: SCOPE,
      enabled,
      getDisplayName: () => "Agent Ada",
      onPulse: (pubkey) => pulses.push(pubkey),
      onTurnOff: () => {},
      onTurnOn: () => turnOnCalls.push(true),
    }),
  );

  return {
    act,
    audience: () => getPersistentAgentAudienceSnapshot().audiences[SCOPE] ?? [],
    pulses,
    result,
    turnOnCalls,
  };
}

test("ordinary explicit address stays one-shot when auto-mention is off", async () => {
  const { act, audience, pulses, result, turnOnCalls } = await renderAutoPin({
    enabled: false,
  });

  act(() => {
    result.current.promoteExplicitlyAddressedAgents({ pubkeys: [AGENT] });
  });

  assert.deepEqual(audience(), []);
  assert.equal(pulses.length, 0);
  assert.equal(turnOnCalls.length, 0);
  assert.equal(result.current.openOptionsRequest, 0);
});

test("always-address persist still opts in when auto-mention is off", async () => {
  const { act, audience, pulses, result, turnOnCalls } = await renderAutoPin({
    enabled: false,
  });

  act(() => {
    result.current.promoteExplicitlyAddressedAgents({
      persist: true,
      pubkeys: [AGENT],
    });
  });

  assert.deepEqual(audience(), [AGENT]);
  assert.deepEqual(pulses, [AGENT]);
  assert.equal(result.current.openOptionsRequest, 1);

  act(() => {
    result.current.completeOptionsReveal(1);
  });
  assert.equal(turnOnCalls.length, 1);
});
