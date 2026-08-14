import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  createDefaultSettings,
  inverseParticleColor,
} from "./customCardParticleGrid.ts";

describe("createDefaultSettings", () => {
  it("keeps the Simon-inspired custom-card Thinking preset stable", () => {
    assert.deepEqual(createDefaultSettings(), {
      gridSize: 15,
      charScale: 1,
      noiseScale: 0.0008,
      noiseSpeed: 0.5,
      idleOpacity: 0.14,
      baseOpacity: 0.3,
      touchRadius: 1175,
      touchStrength: 1.25,
      trailDuration: 4,
      charGamma: 1,
      dotColor: [0, 0, 0],
      backgroundColor: [1, 1, 1],
      mode: 2,
      gridOpacity: 1,
      thinkingFade: 1,
      pullProgress: 0,
      audioLevel: 0,
      audioPhase: 0,
      showSparkles: false,
      charSet: "H",
      objectRevealStyle: 5,
    });
  });

  it("returns an independent settings object", () => {
    const first = createDefaultSettings();
    first.dotColor[0] = 0;

    assert.deepEqual(createDefaultSettings().dotColor, [0, 0, 0]);
  });

  it("derives the grid color as the inverse of the background", () => {
    assert.deepEqual(inverseParticleColor([0.2, 0.65, 1]), [0.8, 0.35, 0]);
  });
});
