/**
 * Tenant scoping and in-flight deduplication for the publish-first detached
 * agent wake.
 *
 * `useMentionSendFlow` no longer awaits `start_managed_agent`, so the call
 * outlives the send — and, because a community switch only remounts the React
 * subtree, it can outlive the community too. Two consequences are pinned here:
 *
 * 1. `start_managed_agent` resolves the workspace relay and signing identity at
 *    execution time, so without a captured scope a wake fired in community A
 *    can spawn/deploy the agent against community B (carrying A's replay
 *    floor).
 * 2. Nothing else dedupes concurrent wakes any more — the awaited version was
 *    covered by the composer's `isPending` gate plus the mutation's success
 *    cache write, and for the whole detached window the cache still reads
 *    `stopped`, so a second send re-fires.
 * 3. A scope that is not yet known is not a scope: the backend reads a missing
 *    relay or signer as "no assertion", so the wake is refused rather than
 *    fired unscoped.
 * 4. The failure warning is fenced to the scope it was fired under: the
 *    `<Toaster />` outlives the community remount boundary, so an unfenced
 *    toast from a start that settled after a switch would name community A's
 *    agent over community B's UI. Delivery compares the captured scope against
 *    the on-screen mirror (`detachedToastScope`) at toast time — not a reset
 *    generation, so an A→B→A round-trip warns again once A is back on screen.
 * 5. The in-flight map outlives a community switch on purpose: the backend's
 *    scope assertion is a current-state check, so an A→B→A round-trip
 *    re-validates a still-held start — the map entry is its only duplicate
 *    guard, and it is tenant-keyed, so retention cannot affect the community
 *    being entered. (`resetCommunityState` no longer clears it; that seam is
 *    pinned E2E.)
 * 6. That dedupe key is the *whole* scope the wake asserts — relay and signer,
 *    not the relay alone. The backend distinguishes signers before it spawns
 *    or deploys, so the renderer must not coalesce across them: a start held
 *    under the identity in force before a mid-session key import cannot stand
 *    in for the imported identity's wake.
 *
 * These tests drive the real hook against the real CommunitiesProvider; the
 * scope mirror is driven directly through its module seam, standing in for
 * `useCommunityInit` (its only production writer), which is not mounted here.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, beforeEach, test } from "node:test";

import { JSDOM } from "jsdom";

const SELF = "1".repeat(64);
// The key a mid-session import puts in force — the signing identity is one
// global per install, so two signers only ever arrive back to back.
const IMPORTED_SELF = "2".repeat(64);
const AGENT = "a".repeat(64);
const OTHER_AGENT = "b".repeat(64);
// Mixed case on purpose: the backend's scope comparison is case-sensitive
// past the scheme, so the stored URL must reach it verbatim.
const RELAY_A = "wss://Tenant-A.example";
const RELAY_B = "wss://tenant-b.example";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** Every `start_managed_agent` payload seen, in call order. */
let startCalls = [];
/**
 * Settlers for `start_managed_agent` calls held open by `holdStarts`, in call
 * order. A held start is what the dedupe exists for: the seconds-long window
 * where a cold spawn or first deploy has not yet updated the agent record.
 */
let heldStarts = [];
let holdStarts = false;
/**
 * Settlers for `get_identity` calls held open by `holdIdentity` — the window
 * before the identity query resolves, where the signer half of the scope is
 * simply not known yet.
 */
let heldIdentity = [];
let holdIdentity = false;
/**
 * The identity `get_identity` currently reports. Mutable so a test can move it
 * the way a mid-session key import does, keeping a refetch consistent with the
 * cache write that drove the switch.
 */
let currentIdentityPubkey = SELF;

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    localStorage: dom.window.localStorage,
    window: dom.window,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (command, args) => {
      if (command === "get_identity") {
        const identity = { pubkey: currentIdentityPubkey, display_name: "Me" };
        if (!holdIdentity) return Promise.resolve(identity);
        return new Promise((resolve) => {
          heldIdentity.push(() => resolve(identity));
        });
      }
      if (command === "start_managed_agent") {
        startCalls.push(args);
        const started = { pubkey: args.pubkey, status: "running" };
        if (!holdStarts) return Promise.resolve(started);
        return new Promise((resolve, reject) => {
          heldStarts.push({
            resolve: () => resolve(started),
            reject: () => reject(new Error("spawn failed")),
          });
        });
      }
      return Promise.reject(new Error(`unmocked Tauri command: ${command}`));
    },
    transformCallback: () => 1,
  };
  globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__;
});

after(() => dom.window.close());

afterEach(async () => {
  // Tests that pin suppression leave their wake deliberately in flight, and
  // node:test never finishes a file with a promise that will not settle. The
  // same goes for an identity query a test deliberately left pending.
  const outstanding = heldStarts;
  heldStarts = [];
  for (const held of outstanding) held.resolve();
  const outstandingIdentity = heldIdentity;
  heldIdentity = [];
  for (const resolveIdentity of outstandingIdentity) resolveIdentity();
  await new Promise((resolve) => setTimeout(resolve, 0));
});

beforeEach(async () => {
  startCalls = [];
  heldStarts = [];
  holdStarts = false;
  heldIdentity = [];
  holdIdentity = false;
  currentIdentityPubkey = SELF;
  // Toasts queue on a module-level store with no <Toaster> mounted, so one
  // test's warning would otherwise be visible to the next.
  const { toast } = await import("sonner");
  toast.dismiss();
  // The in-flight map is a module singleton, so a start held open by one test
  // would otherwise suppress the next test's.
  const { resetDetachedAgentStarts } = await import(
    "./useDetachedAgentStart.ts"
  );
  resetDetachedAgentStarts();
  // The toast-scope mirror is a module singleton too. Point it at community A
  // — what `useCommunityInit` does when A's apply completes — so failure
  // warnings deliver by default, as in the running app.
  const { resetDetachedToastScope, setDetachedToastScope } = await import(
    "@/features/messages/lib/detachedToastScope.ts"
  );
  resetDetachedToastScope();
  setDetachedToastScope({ relayUrl: RELAY_A, signerPubkey: SELF });
  window.localStorage.clear();
  window.localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-a",
        name: "Tenant A",
        relayUrl: RELAY_A,
        pubkey: SELF,
        addedAt: "2026-01-01T00:00:00Z",
      },
      {
        id: "community-b",
        name: "Tenant B",
        relayUrl: RELAY_B,
        pubkey: SELF,
        addedAt: "2026-01-02T00:00:00Z",
      },
    ]),
  );
  window.localStorage.setItem("buzz-active-community-id", "community-a");
});

/**
 * Renders the real hook under the real communities provider, exposing the
 * detached-start callback alongside `switchCommunity` so a test can move the
 * active community out from under an already-captured callback.
 *
 * `act` is returned too, so a test can drive the render that follows a
 * deliberately-delayed identity resolution, and the `QueryClient` so a test can
 * move the signing identity the way a mid-session import does.
 */
async function renderDetachedStart() {
  const { default: React } = await import("react");
  const { act, renderHook } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { CommunitiesProvider, useCommunities } = await import(
    "@/features/communities/useCommunities.tsx"
  );
  const { useIdentityQuery } = await import("@/shared/api/hooks.ts");
  const { useDetachedAgentStart } = await import("./useDetachedAgentStart.ts");

  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0 },
      mutations: { gcTime: 0 },
    },
  });
  const wrapper = ({ children }) =>
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(CommunitiesProvider, null, children),
    );
  const rendered = renderHook(
    () => ({
      identityPubkey: useIdentityQuery().data?.pubkey,
      startDetached: useDetachedAgentStart(),
      switchCommunity: useCommunities().switchCommunity,
    }),
    { wrapper },
  );
  if (!holdIdentity) await waitForIdentity(act, rendered);
  return { act, client, rendered };
}

/**
 * Flushes until the identity query has landed (on `expected`, when given),
 * rather than for a fixed number of ticks: a wake fired before the signer
 * scope resolves is now refused, so an under-wait would surface as a phantom
 * suppression bug.
 */
async function waitForIdentity(act, rendered, expected) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const current = rendered.result.current.identityPubkey;
    if (expected ? current === expected : Boolean(current)) return;
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }
  assert.fail(
    expected
      ? `the identity query never reported ${expected}`
      : "the identity query never resolved",
  );
}

/**
 * Puts a different signing key in force mid-session, through the exact seam
 * production uses: the membership-denied onboarding overlay imports an nsec and
 * writes the new identity straight into the live identity query cache
 * (`CommunityOnboardingFlow`), while the app underneath — module singletons,
 * in-flight mutations, this map — keeps running.
 */
async function importIdentity(act, rendered, client, pubkey) {
  currentIdentityPubkey = pubkey;
  await act(async () => {
    client.setQueryData(["identity"], { pubkey, display_name: "Me" });
  });
  await waitForIdentity(act, rendered, pubkey);
}

const AGENT_RECORD = { pubkey: AGENT, name: "fizz" };
const OTHER_AGENT_RECORD = { pubkey: OTHER_AGENT, name: "buzz" };

/** Lets queued microtasks (the mutation, and the map's `finally`) run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("a detached start carries the active community and identity as its scope", async () => {
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  assert.equal(startCalls.length, 1);
  assert.equal(
    startCalls[0].expectedRelayUrl,
    RELAY_A,
    "the stored relay URL must reach the case-sensitive backend check verbatim",
  );
  assert.equal(startCalls[0].expectedSignerPubkey, SELF);
  // The replay floor still rides along — scoping must not displace it.
  assert.ok(startCalls[0].replayFloorUnix > 0);
  rendered.unmount();
});

test("an explicit replay floor reaches the start payload verbatim", async () => {
  // The send path stamps the floor when it queues the wake (pre-publish, so
  // the floor can never exceed the published message's created_at) and only
  // flushes the wake after the relay accepts the publish — a flush-time
  // stamp could push the harness's startup watermark past that message.
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD, 1_234_567);
    await settle();
  });

  assert.equal(startCalls.length, 1);
  assert.equal(startCalls[0].replayFloorUnix, 1_234_567);
  rendered.unmount();
});

test("a wake with no queued floor captures fire time", async () => {
  const { act, rendered } = await renderDetachedStart();
  const before = Math.floor(Date.now() / 1000);

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  const after = Math.floor(Date.now() / 1000);
  assert.equal(startCalls.length, 1);
  assert.ok(
    startCalls[0].replayFloorUnix >= before &&
      startCalls[0].replayFloorUnix <= after,
    "callers with no queued floor keep the fire-time capture",
  );
  rendered.unmount();
});

test("a start captured before a community switch keeps the pre-switch scope", async () => {
  const { act, rendered } = await renderDetachedStart();
  // What a send in flight holds: the callback from the render that fired it.
  const capturedStart = rendered.result.current.startDetached;

  await act(async () => {
    rendered.result.current.switchCommunity("community-b");
  });
  assert.notEqual(
    rendered.result.current.startDetached,
    capturedStart,
    "the switch must produce a new callback, so the captured one is stale",
  );

  await act(async () => {
    capturedStart(AGENT_RECORD);
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  assert.equal(startCalls.length, 1);
  assert.equal(
    startCalls[0].expectedRelayUrl,
    RELAY_A,
    "the stale wake must name the community it was fired in, not the active one",
  );
  rendered.unmount();
});

test("a second wake for the same agent is suppressed while the first is in flight", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();
  let first;
  let second;

  await act(async () => {
    first = rendered.result.current.startDetached(AGENT_RECORD);
    second = rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(startCalls.length, 1, "one wake serves both messages");
  assert.equal(first, true);
  assert.equal(
    second,
    false,
    "the suppressed call must report that it fired nothing",
  );
  rendered.unmount();
});

test("wakes for different agents in one window are not collapsed", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    rendered.result.current.startDetached(OTHER_AGENT_RECORD);
    await settle();
  });

  assert.deepEqual(
    startCalls.map((call) => call.pubkey),
    [AGENT, OTHER_AGENT],
    "the key is per agent, not a global lock on wakes",
  );
  rendered.unmount();
});

test("a wake fires again once the previous one has settled", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    heldStarts[0].resolve();
    await settle();
  });
  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(startCalls.length, 2, "suppression must not outlive the start");
  rendered.unmount();
});

test("an A→B→A community round-trip does not reopen the in-flight window", async () => {
  // The production seam — `resetCommunityState` deliberately not clearing the
  // map on switch — is pinned E2E, where the real switch path runs. This pins
  // the hook contract that makes retention sufficient: the round-trip hands
  // back a callback carrying A's scope again, its key matches the retained
  // entry, and the wake stays suppressed while the first start (whose scope
  // is valid again now that A is re-applied) is still deploying.
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    rendered.result.current.switchCommunity("community-b");
  });
  await act(async () => {
    rendered.result.current.switchCommunity("community-a");
  });

  let refire;
  await act(async () => {
    refire = rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(refire, false, "the retained entry must still suppress");
  assert.equal(
    startCalls.length,
    1,
    "a round-trip must not duplicate a deploy the first start is still performing",
  );
  rendered.unmount();
});

test("a wake re-fires once the start held across a round-trip has rejected", async () => {
  // Retention ends at settlement, not at some later reset: the `finally`
  // self-cleans, so a failed start never latches the agent — the user's next
  // send after the failure toast gets a real wake.
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    rendered.result.current.switchCommunity("community-b");
  });
  await act(async () => {
    rendered.result.current.switchCommunity("community-a");
  });
  await act(async () => {
    heldStarts[0].reject();
    await settle();
  });
  await act(async () => {
    assert.equal(rendered.result.current.startDetached(AGENT_RECORD), true);
    await settle();
  });

  assert.equal(
    startCalls.length,
    2,
    "settlement must end the suppression even across a round-trip",
  );
  rendered.unmount();
});

test("a failed wake clears the key instead of latching the agent", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    heldStarts[0].reject();
    await settle();
  });
  // The user saw "your message was sent, but the agent may not respond" and
  // retries; clearing on success only would refuse every retry for the session.
  await act(async () => {
    assert.equal(rendered.result.current.startDetached(AGENT_RECORD), true);
    await settle();
  });

  assert.equal(startCalls.length, 2);
  rendered.unmount();
});

/** The titles of every toast raised since the last `beforeEach`. */
async function toastTitles() {
  const { toast } = await import("sonner");
  return toast.getToasts().map((entry) => String(entry.title ?? ""));
}

const MAY_NOT_RESPOND = /your message was sent, but the agent may not respond/;

test("a wake is refused while the identity that would scope it is unresolved", async () => {
  // The window before `get_identity` lands. `expectedSignerPubkey` would be
  // undefined here, and the backend reads that as "no assertion" — so the
  // wake would resolve the signing identity at execution time, which is the
  // cross-tenant spawn the scope exists to prevent.
  holdIdentity = true;
  const { act, rendered } = await renderDetachedStart();

  let fired;
  await act(async () => {
    fired = rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(fired, false, "a refused wake must not be counted as one");
  assert.equal(startCalls.length, 0, "an unscoped wake must not be fired");
  assert.match(
    (await toastTitles()).join("\n"),
    MAY_NOT_RESPOND,
    "publish-first means the message went out; the user has to be told the agent did not wake",
  );

  // The refusal covers this moment, not the session: once the query lands the
  // next send fires a fully scoped wake.
  for (const resolveIdentity of heldIdentity.splice(0)) resolveIdentity();
  await waitForIdentity(act, rendered);
  await act(async () => {
    assert.equal(rendered.result.current.startDetached(AGENT_RECORD), true);
    await settle();
  });

  assert.equal(startCalls.length, 1);
  assert.equal(startCalls[0].expectedRelayUrl, RELAY_A);
  assert.equal(startCalls[0].expectedSignerPubkey, SELF);
  rendered.unmount();
});

test("a wake is refused when no community is active to scope it", async () => {
  window.localStorage.setItem("buzz-communities", JSON.stringify([]));
  window.localStorage.removeItem("buzz-active-community-id");
  const { act, rendered } = await renderDetachedStart();

  let fired;
  await act(async () => {
    fired = rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(fired, false);
  assert.equal(startCalls.length, 0);
  assert.match((await toastTitles()).join("\n"), MAY_NOT_RESPOND);
  rendered.unmount();
});

test("a blank relay URL is refused rather than sent as no assertion", async () => {
  // `assert_expected_relay_scope` discards a whitespace-only scope exactly as
  // it discards a missing one, so emptiness has to be judged on the trimmed
  // form here — a blank stored URL is an unscoped wake, not a scoped one.
  window.localStorage.setItem(
    "buzz-communities",
    JSON.stringify([
      {
        id: "community-blank",
        name: "Blank",
        relayUrl: "   ",
        pubkey: SELF,
        addedAt: "2026-01-01T00:00:00Z",
      },
    ]),
  );
  window.localStorage.setItem("buzz-active-community-id", "community-blank");
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    assert.equal(rendered.result.current.startDetached(AGENT_RECORD), false);
    await settle();
  });

  assert.equal(startCalls.length, 0);
  rendered.unmount();
});

/** The scope-mirror module, driven directly in place of `useCommunityInit`. */
async function toastScopeModule() {
  return import("@/features/messages/lib/detachedToastScope.ts");
}

test("a start failure warns while the community it was fired in is on screen", async () => {
  // The positive control for the fence: with the mirror still pointing at the
  // scope the wake captured (beforeEach models A's completed apply), the
  // failure toast must deliver — a fence that suppresses everything would
  // silently drop the only signal that the agent never woke.
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    heldStarts[0].reject();
    await settle();
  });

  assert.match(
    (await toastTitles()).join("\n"),
    MAY_NOT_RESPOND,
    "an on-scope failure must keep warning the user",
  );
  rendered.unmount();
});

test("a start failure fired in one community stays silent while another is on screen", async (t) => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  // What a real switch does to the mirror: `resetCommunityState` clears it,
  // B's completed apply repoints it. The rejection then lands with B's UI on
  // screen — where a toast naming A's agent would read as a bug in B.
  const { resetDetachedToastScope, setDetachedToastScope } =
    await toastScopeModule();
  resetDetachedToastScope();
  setDetachedToastScope({ relayUrl: RELAY_B, signerPubkey: SELF });

  const warn = t.mock.method(console, "warn", () => {});
  await act(async () => {
    heldStarts[0].reject();
    await settle();
  });

  assert.doesNotMatch(
    (await toastTitles()).join("\n"),
    MAY_NOT_RESPOND,
    "community A's failure must not toast over community B",
  );
  assert.ok(
    warn.mock.calls.some((call) =>
      String(call.arguments[0]).includes(
        "suppressed a start-failure warning for fizz",
      ),
    ),
    "suppression must stay diagnosable in the console",
  );
  rendered.unmount();
});

test("a start failure that settles mid-switch stays silent", async (t) => {
  // The window between `resetCommunityState` and the next apply: no scope is
  // on screen at all, so delivery fails closed exactly like a mismatch.
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  const { resetDetachedToastScope } = await toastScopeModule();
  resetDetachedToastScope();

  const warn = t.mock.method(console, "warn", () => {});
  await act(async () => {
    heldStarts[0].reject();
    await settle();
  });

  assert.doesNotMatch((await toastTitles()).join("\n"), MAY_NOT_RESPOND);
  assert.ok(
    warn.mock.calls.some((call) =>
      String(call.arguments[0]).includes(
        "suppressed a start-failure warning for fizz",
      ),
    ),
  );
  rendered.unmount();
});

test("an A→B→A round-trip keeps the warning deliverable back in A", async () => {
  // The fence compares scopes at toast time — deliberately not "has a reset
  // happened since capture". A generation check would get this wrong: the
  // user is back in A when the slow start settles, the warning concerns the
  // community on screen, and re-mentioning the agent is actionable right
  // there.
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  const { resetDetachedToastScope, setDetachedToastScope } =
    await toastScopeModule();
  resetDetachedToastScope();
  setDetachedToastScope({ relayUrl: RELAY_B, signerPubkey: SELF });
  resetDetachedToastScope();
  setDetachedToastScope({ relayUrl: RELAY_A, signerPubkey: SELF });

  await act(async () => {
    heldStarts[0].reject();
    await settle();
  });

  assert.match(
    (await toastTitles()).join("\n"),
    MAY_NOT_RESPOND,
    "back in A, the warning is on-scope again and must show",
  );
  rendered.unmount();
});

test("a wake for the same agent in another community is not suppressed", async () => {
  holdStarts = true;
  const { act, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await act(async () => {
    rendered.result.current.switchCommunity("community-b");
  });
  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.deepEqual(
    startCalls.map((call) => call.expectedRelayUrl),
    [RELAY_A, RELAY_B],
    "the key carries the relay, so one tenant's in-flight wake never suppresses another's",
  );
  rendered.unmount();
});

test("a wake under a newly imported identity is not suppressed by one held under the old", async () => {
  // The signer half of the same boundary. `start_managed_agent` asserts the
  // expected signer before it spawns or deploys — and a provider deploy
  // re-asserts it against the payload rebuilt after the deploy lock — so a
  // start held under the previous identity is not the wake this one is owed.
  // Suppressing it would drop every send for the length of a cold spawn or
  // first deploy, and leave the agent deployed under the stale owner.
  holdStarts = true;
  const { act, client, rendered } = await renderDetachedStart();

  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  await importIdentity(act, rendered, client, IMPORTED_SELF);

  let refire;
  await act(async () => {
    refire = rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });

  assert.equal(refire, true, "the identity now in force is owed its own wake");
  assert.deepEqual(
    startCalls.map((call) => call.expectedSignerPubkey),
    [SELF, IMPORTED_SELF],
    "the key carries the signer, so one identity's in-flight wake never suppresses another's",
  );
  // Same agent on the same relay: the signer is the only thing separating the
  // two operations, which is exactly what the pre-fix key could not see.
  assert.deepEqual(
    startCalls.map((call) => call.pubkey),
    [AGENT, AGENT],
  );
  assert.deepEqual(
    startCalls.map((call) => call.expectedRelayUrl),
    [RELAY_A, RELAY_A],
  );
  rendered.unmount();
});

test("settling one signer's start leaves the other signer's suppression intact", async () => {
  // Per-signer settlement independence: the `finally` delete closes over the
  // key it registered, so freeing the imported identity's entry must not lift
  // the suppression the still-held old-identity start is providing (nor the
  // reverse).
  holdStarts = true;
  const { act, client, rendered } = await renderDetachedStart();
  // What a send fired before the import holds: the callback from the render
  // that fired it, still carrying the old signer.
  const startAsOldIdentity = rendered.result.current.startDetached;

  await act(async () => {
    startAsOldIdentity(AGENT_RECORD);
    await settle();
  });
  await importIdentity(act, rendered, client, IMPORTED_SELF);
  await act(async () => {
    rendered.result.current.startDetached(AGENT_RECORD);
    await settle();
  });
  assert.equal(startCalls.length, 2, "both signers have a start in flight");

  await act(async () => {
    heldStarts[1].resolve();
    await settle();
  });

  let refireImported;
  let refireOld;
  await act(async () => {
    refireImported = rendered.result.current.startDetached(AGENT_RECORD);
    refireOld = startAsOldIdentity(AGENT_RECORD);
    await settle();
  });

  assert.equal(refireImported, true, "the settled signer's key is free again");
  assert.equal(
    refireOld,
    false,
    "the still-held signer's entry must survive another signer's settlement",
  );
  assert.equal(startCalls.length, 3);
  assert.equal(startCalls[2].expectedSignerPubkey, IMPORTED_SELF);
  rendered.unmount();
});
