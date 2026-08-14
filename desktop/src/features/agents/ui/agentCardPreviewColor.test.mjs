import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  fallbackAgentCardColor,
  sampleRepresentativeAvatarColor,
} from "./agentCardPreviewColor.ts";

describe("sampleRepresentativeAvatarColor", () => {
  it("ignores transparent pixels", () => {
    const pixels = new Uint8ClampedArray([10, 90, 210, 255, 250, 30, 30, 0]);

    assert.equal(sampleRepresentativeAvatarColor(pixels), "rgb(10 90 210)");
  });

  it("prefers a distinctive colorful subject over a neutral background", () => {
    const pixels = new Uint8ClampedArray([
      120, 120, 120, 255, 122, 122, 122, 255, 30, 180, 150, 255,
    ]);

    assert.equal(sampleRepresentativeAvatarColor(pixels), "rgb(30 180 150)");
  });

  it("uses the most prominent hue instead of a smaller saturated accent", () => {
    const green = [48, 178, 76, 255];
    const fuchsia = [245, 0, 220, 255];
    const pixels = new Uint8ClampedArray([
      ...green,
      ...green,
      ...green,
      ...green,
      ...green,
      ...green,
      ...green,
      ...green,
      ...fuchsia,
      ...fuchsia,
    ]);

    assert.equal(sampleRepresentativeAvatarColor(pixels), "rgb(48 178 76)");
  });

  it("falls back to a representative neutral for monochrome avatars", () => {
    const pixels = new Uint8ClampedArray([90, 90, 90, 255, 150, 150, 150, 255]);

    assert.equal(sampleRepresentativeAvatarColor(pixels), "rgb(121 121 121)");
  });

  it("returns null when no visible midtone pixels can be sampled", () => {
    const pixels = new Uint8ClampedArray([
      0, 0, 0, 255, 255, 255, 255, 255, 20, 40, 60, 0,
    ]);

    assert.equal(sampleRepresentativeAvatarColor(pixels), null);
  });
});

describe("fallbackAgentCardColor", () => {
  it("is stable and varies across agent ids", () => {
    assert.equal(
      fallbackAgentCardColor("agent-a"),
      fallbackAgentCardColor("agent-a"),
    );
    assert.notEqual(
      fallbackAgentCardColor("agent-a"),
      fallbackAgentCardColor("agent-b"),
    );
  });
});
