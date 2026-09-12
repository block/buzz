import assert from "node:assert/strict";
import test from "node:test";

import { describeAgentDevice } from "./agentDeviceLabel.ts";

test("device label stays silent for a local agent with no name collision", () => {
  assert.equal(
    describeAgentDevice({
      isLocal: true,
      deviceLabel: "this-mac",
      hasNameCollision: false,
    }),
    null,
  );
  assert.equal(
    describeAgentDevice({
      isLocal: true,
      deviceLabel: null,
      hasNameCollision: false,
    }),
    null,
  );
});

test("device label names this device when a local agent collides on name", () => {
  assert.equal(
    describeAgentDevice({
      isLocal: true,
      deviceLabel: "mfeth-win",
      hasNameCollision: true,
    }),
    "on this device",
  );
  assert.equal(
    describeAgentDevice({
      isLocal: true,
      deviceLabel: null,
      hasNameCollision: true,
    }),
    "on this device",
  );
});

test("device label names the remote device when one is published", () => {
  assert.equal(
    describeAgentDevice({
      isLocal: false,
      deviceLabel: "mfeth-win",
      hasNameCollision: true,
    }),
    "on mfeth-win",
  );
  assert.equal(
    describeAgentDevice({
      isLocal: false,
      deviceLabel: "mfeth-win",
      hasNameCollision: false,
    }),
    "on mfeth-win",
  );
  assert.equal(
    describeAgentDevice({
      isLocal: false,
      deviceLabel: "  mfeth-win  ",
      hasNameCollision: false,
    }),
    "on mfeth-win",
  );
});

test("device label never fabricates a name for a remote agent without one", () => {
  for (const deviceLabel of [null, undefined, "", "   ", "\t\n"]) {
    assert.equal(
      describeAgentDevice({
        isLocal: false,
        deviceLabel,
        hasNameCollision: false,
      }),
      "on another device",
    );
  }
});
