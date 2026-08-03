import assert from "node:assert/strict";
import test from "node:test";

import {
  applyRelayConnectionPolicy,
  isRelayUrlAllowed,
} from "./relayPolicy.ts";

const locked = {
  lockedToDefaultRelay: true,
  defaultRelayUrl: "wss://buzz.block.builderlab.xyz",
};

test("locked policy compares exact normalized origins", () => {
  assert.equal(
    isRelayUrlAllowed(locked, "wss://BUZZ.block.builderlab.xyz/"),
    true,
  );
  for (const relayUrl of [
    "ws://buzz.block.builderlab.xyz",
    "wss://public.example.com",
    "wss://public.buzz.block.builderlab.xyz",
    "wss://buzz.block.builderlab.xyz:8443",
    "wss://buzz.block.builderlab.xyz/path",
  ]) {
    assert.equal(isRelayUrlAllowed(locked, relayUrl), false, relayUrl);
  }
});

test("unrestricted policy preserves existing behavior", () => {
  assert.equal(
    isRelayUrlAllowed(
      {
        lockedToDefaultRelay: false,
        defaultRelayUrl: "ws://localhost:3000",
      },
      "not a URL",
    ),
    true,
  );
});

test("locked policy removes saved external communities before app mount", () => {
  const communities = [
    { id: "external", relayUrl: "wss://public.example.com" },
    { id: "block", relayUrl: "wss://buzz.block.builderlab.xyz" },
  ];
  assert.deepEqual(
    applyRelayConnectionPolicy(communities, "external", locked),
    {
      communities: [communities[1]],
      activeId: "block",
      changed: true,
    },
  );
});

test("locked policy clears state when no saved community is allowed", () => {
  const communities = [
    { id: "external", relayUrl: "wss://public.example.com" },
  ];
  assert.deepEqual(
    applyRelayConnectionPolicy(communities, "external", locked),
    { communities: [], activeId: null, changed: true },
  );
});
