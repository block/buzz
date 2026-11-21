import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { compileScene } from "./activityScene.ts";
import { drawFrame } from "./renderScene.ts";
import {
  chooseActivityGridStep,
  formatActivityDuration,
  getActivityPanelLayout,
  timeToCanvasX,
} from "./renderSceneLayout.ts";

function gridTickCount(durationMs, chartWidthPx) {
  return (
    Math.floor(durationMs / chooseActivityGridStep(durationMs, chartWidthPx)) +
    1
  );
}

function sceneWithDisplayRange(d0, d1) {
  return { d0, d1 };
}

const T0 = Date.UTC(2026, 7, 10, 12, 0, 0);

class RecordingCanvasContext {
  operations = [];
  fillStyle = "";
  strokeStyle = "";
  lineWidth = 1;
  font = "";
  textAlign = "left";
  textBaseline = "alphabetic";
  globalAlpha = 1;
  lineCap = "butt";
  lineJoin = "miter";

  record(name, args = []) {
    this.operations.push({ name, args });
  }

  setTransform(...args) {
    this.record("setTransform", args);
  }

  clearRect(...args) {
    this.record("clearRect", args);
  }

  fillRect(...args) {
    this.record("fillRect", args);
  }

  fillText(...args) {
    this.record("fillText", args);
  }

  beginPath() {
    this.record("beginPath");
  }

  moveTo(...args) {
    this.record("moveTo", args);
  }

  lineTo(...args) {
    this.record("lineTo", args);
  }

  closePath() {
    this.record("closePath");
  }

  stroke() {
    this.record("stroke");
  }

  fill() {
    this.record("fill");
  }

  arc(...args) {
    this.record("arc", args);
  }

  save() {
    this.record("save");
  }

  restore() {
    this.record("restore");
  }

  translate(...args) {
    this.record("translate", args);
  }

  rotate(...args) {
    this.record("rotate", args);
  }

  quadraticCurveTo(...args) {
    this.record("quadraticCurveTo", args);
  }

  bezierCurveTo(...args) {
    this.record("bezierCurveTo", args);
  }

  setLineDash(...args) {
    this.record("setLineDash", args);
  }

  createLinearGradient() {
    this.record("createLinearGradient");
    return { addColorStop: () => {} };
  }

  createRadialGradient() {
    this.record("createRadialGradient");
    return { addColorStop: () => {} };
  }
}

function idleGapScene() {
  const events = [
    { agentId: "agent", t: T0, kind: "s" },
    { agentId: "agent", t: T0 + 60_000, kind: "s" },
    { agentId: "agent", t: T0 + 120_000, kind: "s" },
  ];
  const agents = [{ id: "agent", name: "Goose", color: "#7bd88f", spawnT: T0 }];
  return compileScene(events, agents);
}

describe("activity render time helpers", () => {
  it("formats durations without minute overflow", () => {
    assert.equal(formatActivityDuration(59_999), "00:59");
    assert.equal(formatActivityDuration(80_000), "01:20");
    assert.equal(formatActivityDuration(82_072_000), "22:47:52");
  });

  it("chooses grid steps by rounding up from duration and chart width", () => {
    assert.equal(chooseActivityGridStep(80_000, 800), 10_000);
    assert.equal(chooseActivityGridStep(22_500, 400), 5_000);
    assert.equal(chooseActivityGridStep(3 * 60 * 60_000, 1_000), 15 * 60_000);
    assert.equal(chooseActivityGridStep(3 * 60 * 60_000, 200), 2 * 60 * 60_000);
  });

  it("reduces grid tick density on narrow charts", () => {
    const eighteenMinutes = 18 * 60_000;

    assert.ok(gridTickCount(eighteenMinutes, 200) <= 3);
    assert.ok(gridTickCount(eighteenMinutes, 400) <= 6);
    assert.ok(gridTickCount(eighteenMinutes, 800) <= 12);
  });

  it("maps display-space times to chart edges using d0/d1", () => {
    const scene = sceneWithDisplayRange(5_000, 15_000);
    const layout = getActivityPanelLayout(640, 320, 2);

    assert.equal(timeToCanvasX(scene, 5_000, layout), layout.chartX0);
    assert.equal(timeToCanvasX(scene, 15_000, layout), layout.chartX1);
    assert.equal(timeToCanvasX(scene, 20_000, layout), layout.chartX1);
  });
});

describe("activity gap rendering", () => {
  it("uses axis-strip hit targets for idle gaps", () => {
    const scene = idleGapScene();
    const ctx = new RecordingCanvasContext();
    const targets = drawFrame(ctx, scene, {
      viewT: scene.d1,
      liveT: scene.d1,
      width: 640,
      height: 160,
      dpr: 1,
    });
    const layout = getActivityPanelLayout(640, 160, scene.nodes.length);

    assert.equal(targets.gaps.length, scene.timeMap.gaps.length);
    assert.equal(
      targets.gaps.every((target) => target.y === 0),
      true,
    );
    assert.equal(
      targets.gaps.every((target) => target.height === layout.axisHeight),
      true,
    );
    assert.equal(
      targets.gaps.every(
        (target) => target.y + target.height <= layout.axisHeight,
      ),
      true,
    );
    assert.equal(
      targets.gaps.every((target) => target.width >= 8),
      true,
    );
  });

  it("coalesces tight runs of status events into one counted marker", () => {
    const events = [
      // A file touch so the scene has nodes and the agent gets a thread.
      { agentId: "agent", t: T0, kind: "r", path: "README.md" },
      { agentId: "agent", t: T0 + 20, kind: "s", label: "developer.shell" },
      { agentId: "agent", t: T0 + 40, kind: "s", label: "developer.shell" },
      { agentId: "agent", t: T0 + 80, kind: "s", label: "developer.shell" },
      // Far-away status event lands well outside the cluster window.
      { agentId: "agent", t: T0 + 60_000, kind: "s", label: "developer.shell" },
    ];
    const agents = [
      { id: "agent", name: "Goose", color: "#7bd88f", spawnT: T0 },
    ];
    const scene = compileScene(events, agents);
    const ctx = new RecordingCanvasContext();

    const targets = drawFrame(ctx, scene, {
      viewT: scene.d1,
      liveT: scene.d1,
      width: 640,
      height: 160,
      dpr: 1,
    });

    const statusTargets = targets.events.filter(
      (target) => target.event.kind === "s",
    );
    assert.equal(statusTargets.length, 2);

    const cluster = statusTargets.find((target) => target.clusterCount === 3);
    assert.notEqual(cluster, undefined);
    assert.deepEqual(cluster?.clusterLabels, ["developer.shell"]);

    const countLabels = ctx.operations.filter(
      (op) => op.name === "fillText" && op.args[0] === "×3",
    );
    assert.equal(countLabels.length, 1);
  });

  it("keeps distant status events as separate markers", () => {
    const events = [
      // A file touch so the scene has nodes and the agent gets a thread.
      { agentId: "agent", t: T0, kind: "r", path: "README.md" },
      { agentId: "agent", t: T0 + 100, kind: "s", label: "developer.shell" },
      { agentId: "agent", t: T0 + 10_000, kind: "s", label: "developer.shell" },
    ];
    const agents = [
      { id: "agent", name: "Goose", color: "#7bd88f", spawnT: T0 },
    ];
    const scene = compileScene(events, agents);
    const ctx = new RecordingCanvasContext();

    const targets = drawFrame(ctx, scene, {
      viewT: scene.d1,
      liveT: scene.d1,
      width: 640,
      height: 160,
      dpr: 1,
    });

    const statusTargets = targets.events.filter(
      (target) => target.event.kind === "s",
    );
    assert.equal(statusTargets.length, 2);
    assert.equal(
      statusTargets.every((target) => !target.clusterCount),
      true,
    );
  });

  it("draws one axis tick/bracket stroke per idle gap", () => {
    const scene = idleGapScene();
    const ctx = new RecordingCanvasContext();

    drawFrame(ctx, scene, {
      viewT: scene.d1,
      liveT: scene.d1,
      width: 640,
      height: 160,
      dpr: 1,
    });

    assert.equal(
      ctx.operations.filter((op) => op.name === "stroke").length,
      scene.timeMap.gaps.length,
    );
  });
});

describe("rider hit targets", () => {
  it("emits one rider target at the playhead for a single live agent", () => {
    const events = [
      { agentId: "agent", t: T0, kind: "r", path: "README.md" },
      { agentId: "agent", t: T0 + 10_000, kind: "r", path: "README.md" },
    ];
    const agents = [
      { id: "agent", name: "Goose", color: "#7bd88f", spawnT: T0 },
    ];
    const scene = compileScene(events, agents);
    const ctx = new RecordingCanvasContext();

    const targets = drawFrame(ctx, scene, {
      viewT: scene.d1,
      liveT: scene.d1,
      width: 640,
      height: 160,
      dpr: 1,
    });
    const layout = getActivityPanelLayout(640, 160, scene.nodes.length);

    assert.equal(targets.riders.length, 1);
    const rider = targets.riders[0];
    assert.equal(rider.kind, "rider");
    assert.equal(rider.id, "rider:agent");
    assert.equal(rider.agentId, "agent");
    assert.equal(rider.agentName, "Goose");
    assert.equal(rider.x, timeToCanvasX(scene, scene.d1, layout));
    assert.ok(rider.radius >= 6);
  });

  it("emits one rider target per live agent", () => {
    const events = [
      { agentId: "goose", t: T0, kind: "r", path: "README.md" },
      { agentId: "scout", t: T0 + 10_000, kind: "w", path: "src/main.ts" },
    ];
    const agents = [
      { id: "goose", name: "Goose", color: "#7bd88f", spawnT: T0 },
      { id: "scout", name: "Scout", color: "#6fb7ff", spawnT: T0 + 10_000 },
    ];
    const scene = compileScene(events, agents);
    const ctx = new RecordingCanvasContext();

    const targets = drawFrame(ctx, scene, {
      viewT: scene.d1,
      liveT: scene.d1,
      width: 640,
      height: 160,
      dpr: 1,
    });

    assert.equal(targets.riders.length, 2);
    assert.deepEqual(targets.riders.map((target) => target.agentId).sort(), [
      "goose",
      "scout",
    ]);
  });

  it("excludes riders for agents that have not spawned at viewT", () => {
    const events = [
      { agentId: "goose", t: T0, kind: "r", path: "README.md" },
      { agentId: "scout", t: T0 + 20_000, kind: "w", path: "src/main.ts" },
    ];
    const agents = [
      { id: "goose", name: "Goose", color: "#7bd88f", spawnT: T0 },
      { id: "scout", name: "Scout", color: "#6fb7ff", spawnT: T0 + 20_000 },
    ];
    const scene = compileScene(events, agents);
    const scout = scene.agentsById.get("scout");
    // Strictly after goose's spawn (display 0) and before scout's spawn.
    const viewT = scout.displaySpawnT / 2;
    const ctx = new RecordingCanvasContext();

    const targets = drawFrame(ctx, scene, {
      viewT,
      liveT: scene.d1,
      width: 640,
      height: 160,
      dpr: 1,
    });

    assert.equal(targets.riders.length, 1);
    assert.equal(targets.riders[0].agentId, "goose");
  });
});
