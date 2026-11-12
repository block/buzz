import assert from "node:assert/strict";
import { mock, test } from "node:test";
import React, { act } from "react";
import { cleanup, fireEvent, render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { CommunitySwitcher } from "./CommunitySwitcher.tsx";

const community = {
  id: "alpha",
  name: "Alpha",
  relayUrl: "wss://alpha.example",
  addedAt: "2026-01-01T00:00:00Z",
};

function renderSwitcher(t) {
  const client = new QueryClient();
  client.setQueryData(["communityIcon", community.relayUrl], null);
  const view = render(
    React.createElement(
      QueryClientProvider,
      { client },
      React.createElement(CommunitySwitcher, {
        activeCommunity: community,
        communities: [community],
        variant: "profile-menu",
        onSwitchCommunity() {},
        onAddCommunity() {},
        onUpdateCommunity() {},
        async onLeaveCommunity() {},
        async onRemoveCommunityFromDevice() {},
      }),
    ),
  );
  t.after(() => {
    cleanup();
    client.clear();
    mock.timers.reset();
  });
  mock.timers.enable({ apis: ["setTimeout"] });
  return view;
}

test("explicitly opening community actions cancels a pending hover close", (t) => {
  const view = renderSwitcher(t);
  const trigger = view.getByTestId("community-switcher");
  fireEvent.mouseEnter(trigger);
  fireEvent.mouseLeave(trigger);
  fireEvent.click(trigger);
  act(() => mock.timers.tick(160));
  assert.equal(trigger.getAttribute("aria-expanded"), "true");
  assert.ok(view.getByRole("menu", { name: "Community actions" }));
});

test("selecting a community action cancels a pending hover open", (t) => {
  const view = renderSwitcher(t);
  const trigger = view.getByTestId("community-switcher");
  fireEvent.click(trigger);
  fireEvent.mouseEnter(view.getByRole("menu", { name: "Community actions" }));
  fireEvent.click(view.getByRole("menuitem", { name: "Add a community" }));
  act(() => mock.timers.tick(80));
  assert.equal(trigger.getAttribute("aria-expanded"), "false");
  assert.equal(view.queryByRole("menu", { name: "Community actions" }), null);
});
