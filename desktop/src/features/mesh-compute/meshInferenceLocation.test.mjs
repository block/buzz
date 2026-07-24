import assert from "node:assert/strict";
import test from "node:test";

import { describeMeshInferenceLocation } from "./meshInferenceLocation.ts";

const target = (overrides = {}) => ({
  modelId: "org/qwen3-8b:q4",
  modelName: "Qwen3 8B",
  endpointAddr: "addr-a",
  nodeName: null,
  capacity: null,
  endpointId: "endpoint-a",
  deviceId: "device-a",
  deviceName: "Mac Studio",
  ...overrides,
});

const availability = (targets, models = []) => ({
  reason: null,
  models,
  serveTargets: targets,
});

test("null availability (loading/error) renders nothing", () => {
  assert.equal(
    describeMeshInferenceLocation({ availability: null, model: "auto" }),
    null,
  );
});

test("explicit model counts only nodes serving that model", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([
      target(),
      target({ deviceId: "device-b", endpointAddr: "addr-b" }),
      target({
        modelId: "org/other:q4",
        modelName: "Other",
        deviceId: "device-c",
      }),
    ]),
    model: "org/qwen3-8b:q4",
  });
  assert.equal(result.nodeCount, 2);
  assert.equal(result.label, "Shared compute · Qwen3 8B · 2 nodes");
  assert.match(result.title, /Qwen3 8B on 2 nodes serving on this relay/);
});

test("model matching drops the implicit @main revision (Rust parity)", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([target({ modelId: "org/qwen3-8b@main:q4" })]),
    model: "org/qwen3-8b:q4",
  });
  assert.equal(result.nodeCount, 1);
});

test("auto model counts every distinct serving node", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([
      target(),
      target({ modelId: "org/other:q4", deviceId: "device-b" }),
    ]),
    model: "auto",
  });
  assert.equal(result.nodeCount, 2);
  assert.equal(result.label, "Shared compute · auto-routed · 2 nodes");
});

test("empty model behaves like auto", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([target()]),
    model: null,
  });
  assert.equal(result.label, "Shared compute · auto-routed · 1 node");
});

test("same node advertising two models is one node, not two", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([target(), target({ modelId: "org/other:q4" })]),
    model: "auto",
  });
  assert.equal(result.nodeCount, 1);
});

test("falls back to endpoint identity when deviceId is absent", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([
      target({ deviceId: null, endpointId: null }),
      target({ deviceId: null, endpointId: null, endpointAddr: "addr-b" }),
    ]),
    model: "auto",
  });
  assert.equal(result.nodeCount, 2);
});

test("no live nodes yields the explicit none-live state", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([]),
    model: "org/qwen3-8b:q4",
  });
  assert.equal(result.nodeCount, 0);
  assert.equal(result.label, "Shared compute · no live serving nodes");
});

test("model without a display name falls back to the ref", () => {
  const result = describeMeshInferenceLocation({
    availability: availability([target({ modelName: null })]),
    model: "org/qwen3-8b:q4",
  });
  assert.equal(result.label, "Shared compute · org/qwen3-8b:q4 · 1 node");
});
