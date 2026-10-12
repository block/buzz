// A community switch must not leave the relay client on the previous
// community's relay.
//
// History: Settings → Invite to community published a kind:9030 admin add to
// a self-hosted community while a hosted community was selected. The relay
// client read its URL from the backend's process-global override. A relay call
// landing after `disconnect()` but before `apply_workspace` installed the new
// override connected to the OLD relay, and that socket survived as the current
// connection, so every later publish (relay admin events included) went to the
// old community.
import assert from "node:assert/strict";
import { after, before, test } from "node:test";
import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
const selfHosted = {
  id: "self-hosted",
  relayUrl: "wss://self-hosted.example",
  name: "Self-hosted",
};
const hosted = {
  id: "hosted",
  relayUrl: "wss://hosted.example",
  name: "Hosted",
};

let renderHook, waitFor, cleanup, act, useCommunityInit, relayClient;
let backendRelayOverride = null;
let heldApply = null;
let applyCalls = 0;
let heldUrlLookup = null;
const socketOpens = [];

function deferred() {
  let resolve;
  const promise = new Promise((promiseResolve) => {
    resolve = promiseResolve;
  });
  return { promise, resolve };
}

before(async () => {
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    localStorage: dom.window.localStorage,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: async (command, args) => {
      switch (command) {
        case "get_identity":
          return { pubkey: "a".repeat(64) };
        case "apply_workspace":
          applyCalls += 1;
          if (heldApply) await heldApply.promise;
          backendRelayOverride = args.relayUrl;
          return undefined;
        case "get_relay_ws_url": {
          const url = backendRelayOverride;
          if (heldUrlLookup) await heldUrlLookup.promise;
          return url;
        }
        case "plugin:websocket|connect": {
          // The test only needs to know which relay each socket was opened
          // to; the handshake never completes.
          const open = { url: args.url, ...deferred() };
          socketOpens.push(open);
          return open.promise;
        }
        default:
          return undefined;
      }
    },
    transformCallback: () => 1,
  };
  globalThis.__TAURI_INTERNALS__ = dom.window.__TAURI_INTERNALS__;
  ({ renderHook, waitFor, cleanup, act } = await import(
    "@testing-library/react"
  ));
  ({ useCommunityInit } = await import("./useCommunityInit.ts"));
  ({ relayClient } = await import("@/shared/api/relayClient"));
});

after(() => {
  cleanup();
  relayClient.disconnect();
  dom.window.close();
});

test("a relay call during a community switch does not leave the client on the old relay", async () => {
  const { result, rerender } = renderHook(
    ({ community }) => useCommunityInit(community, community.id, false),
    { initialProps: { community: selfHosted } },
  );
  await waitFor(() => assert.equal(result.current.isReady, true));
  assert.equal(backendRelayOverride, selfHosted.relayUrl);

  // Switch to the hosted community, holding the backend apply open so a relay
  // call lands between disconnect() and the new override.
  heldApply = deferred();
  rerender({ community: hosted });
  await waitFor(() => assert.equal(applyCalls, 2));
  void relayClient.preconnect().catch(() => {});
  await waitFor(() => assert.equal(socketOpens.length, 1));
  assert.equal(
    socketOpens[0].url,
    selfHosted.relayUrl,
    "precondition: the gap connect reads the not-yet-replaced override",
  );

  await act(async () => heldApply.resolve());
  heldApply = null;
  await waitFor(() => assert.equal(result.current.isReady, true));
  // The stale attempt finishes opening its socket after the switch.
  await act(async () => socketOpens[0].resolve(1));

  assert.equal(
    relayClient.relayUrl,
    hosted.relayUrl,
    "after the switch the relay client must target the selected community",
  );
  assert.equal(
    relayClient.wsId,
    null,
    "the socket opened to the previous community must not survive the switch",
  );
});

test("a relay URL lookup that outlives a community switch does not overwrite the new relay", async () => {
  cleanup();
  relayClient.disconnect();
  socketOpens.length = 0;
  applyCalls = 0;
  const { result, rerender } = renderHook(
    ({ community }) => useCommunityInit(community, community.id, false),
    { initialProps: { community: selfHosted } },
  );
  await waitFor(() => assert.equal(result.current.isReady, true));

  heldApply = deferred();
  rerender({ community: hosted });
  await waitFor(() => assert.equal(applyCalls, 2));
  // The gap connect reads the outgoing override, but its answer only arrives
  // after the switch has finished.
  heldUrlLookup = deferred();
  void relayClient.preconnect().catch(() => {});
  await act(async () => heldApply.resolve());
  heldApply = null;
  await waitFor(() => assert.equal(result.current.isReady, true));
  await act(async () => heldUrlLookup.resolve());
  heldUrlLookup = null;

  assert.equal(relayClient.relayUrl, hosted.relayUrl);
  assert.equal(
    socketOpens.length,
    0,
    "no socket may be opened to the previous community's relay",
  );
});
