import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { JSDOM } from "jsdom";
import { FixedCommunityProvider, useCommunities } from "./useCommunities.tsx";

test("remote mounted community cannot switch, rewrite custody, or remove its active scope", async () => {
  const dom = new JSDOM("<div id='root'></div>", { url: "https://buzz.test" });
  Object.assign(globalThis, {
    window: dom.window,
    document: dom.window.document,
    IS_REACT_ACT_ENVIRONMENT: true,
  });
  const original = '[{"id":"local","token":"local-custody"}]';
  window.localStorage.setItem("buzz-communities", original);
  const community = {
    id: "remote",
    name: "Staging",
    relayUrl: "wss://staging",
    pubkey: "remote-owner",
  };
  let scope;
  function Probe() {
    scope = useCommunities();
    return null;
  }
  const root = createRoot(document.getElementById("root"));
  try {
    await act(async () =>
      root.render(
        React.createElement(
          FixedCommunityProvider,
          { community },
          React.createElement(Probe),
        ),
      ),
    );
    assert.deepEqual(scope.communities, [community]);
    for (const action of [
      () => scope.switchCommunity("local"),
      () => scope.addCommunity({ ...community, id: "other" }),
      () => scope.removeCommunity("remote"),
      () => scope.clearCommunities(),
      () =>
        scope.updateCommunity("remote", {
          relayUrl: "wss://other",
          token: "nsec",
        }),
      () => scope.reconnectCommunity(),
      () => scope.reorderCommunities(["other"]),
    ])
      assert.throws(action, /Sign out/);
    assert.equal(scope.activeCommunity, community);
    assert.equal(window.localStorage.getItem("buzz-communities"), original);
  } finally {
    await act(async () => root.unmount());
    dom.window.close();
  }
});
