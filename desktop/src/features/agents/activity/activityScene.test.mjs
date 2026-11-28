import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { compileScene, nodeHeat } from "./activityScene.ts";
import { getActivityPanelLayout } from "./renderSceneLayout.ts";

const T0 = Date.UTC(2026, 7, 10, 12, 0, 0);

function ev(t, kind, path, agentId = "agent") {
  return { agentId, t: T0 + t, kind, ...(path ? { path } : {}) };
}

function agent(id = "agent", spawnOffset = 0) {
  return {
    id,
    name: id,
    color: id === "agent" ? "#7bd88f" : "#c792ea",
    spawnT: T0 + spawnOffset,
  };
}

describe("compileScene", () => {
  it("expands ancestors and sorts nodes in explorer order", () => {
    const scene = compileScene(
      [
        ev(1, "r", "src/zeta.ts"),
        ev(2, "r", "src/components/Button.tsx"),
        ev(3, "r", "README.md"),
        ev(4, "r", "docs/guide.md"),
      ],
      [agent()],
    );

    assert.deepEqual(
      scene.nodes.map((node) => `${node.isDir ? "d" : "f"}:${node.path}`),
      [
        "d:docs",
        "f:docs/guide.md",
        "d:src",
        "d:src/components",
        "f:src/components/Button.tsx",
        "f:src/zeta.ts",
        "f:README.md",
      ],
    );
  });

  it("collapses single-child directory chains into one displayed row", () => {
    const path = "src/features/agent-activity-panel/ui/__tests__/x.test.ts";
    const scene = compileScene([ev(1, "r", path)], [agent()]);

    assert.deepEqual(
      scene.nodes.map((node) => ({
        path: node.path,
        name: node.name,
        depth: node.depth,
        isDir: node.isDir,
      })),
      [
        {
          path: "src/features/agent-activity-panel/ui/__tests__",
          name: "src/features/agent-activity-panel/ui/__tests__",
          depth: 0,
          isDir: true,
        },
        { path, name: "x.test.ts", depth: 1, isDir: false },
      ],
    );
    assert.equal(scene.events[0].node, scene.byPath.get(path));
  });

  it("does not collapse through branching directories", () => {
    const scene = compileScene(
      [ev(1, "r", "src/features/a.ts"), ev(2, "r", "src/utils/b.ts")],
      [agent()],
    );

    assert.deepEqual(
      scene.nodes.map((node) => ({
        path: node.path,
        name: node.name,
        depth: node.depth,
        isDir: node.isDir,
      })),
      [
        { path: "src", name: "src", depth: 0, isDir: true },
        { path: "src/features", name: "features", depth: 1, isDir: true },
        { path: "src/features/a.ts", name: "a.ts", depth: 2, isDir: false },
        { path: "src/utils", name: "utils", depth: 1, isDir: true },
        { path: "src/utils/b.ts", name: "b.ts", depth: 2, isDir: false },
      ],
    );
  });

  it("keeps event-terminal directory paths as displayed rows", () => {
    const scene = compileScene(
      [ev(1, "r", "src"), ev(2, "r", "src/features/agent/file.ts")],
      [agent()],
    );

    assert.deepEqual(
      scene.nodes.map((node) => ({
        path: node.path,
        name: node.name,
        depth: node.depth,
        isDir: node.isDir,
      })),
      [
        { path: "src", name: "src", depth: 0, isDir: true },
        {
          path: "src/features/agent",
          name: "features/agent",
          depth: 1,
          isDir: true,
        },
        {
          path: "src/features/agent/file.ts",
          name: "file.ts",
          depth: 2,
          isDir: false,
        },
      ],
    );
    assert.equal(scene.byPath.get("src")?.isDir, true);
    assert.equal(
      scene.events.find((event) => event.path === "src")?.node,
      scene.byPath.get("src"),
    );
  });

  it("assigns reveal times for creates and ancestor directories", () => {
    const scene = compileScene(
      [ev(1, "r", "README.md"), ev(5, "c", "src/new/file.ts")],
      [agent()],
    );

    assert.equal(scene.byPath.get("README.md")?.revealAt, T0);
    assert.equal(scene.byPath.get("src/new/file.ts")?.revealAt, T0 + 5);
    assert.equal(
      scene.nodes.some((node) => node.path === "src"),
      false,
    );
    assert.equal(scene.byPath.get("src/new")?.revealAt, T0 + 5);
  });

  it("synthesizes contention for two agents writing the same path within four seconds", () => {
    const scene = compileScene(
      [
        ev(1_000, "w", "src/store.ts", "agent"),
        ev(4_500, "w", "src/store.ts", "delegate"),
      ],
      [agent("agent"), agent("delegate")],
    );

    const events = scene.events.filter(
      (event) => event.path === "src/store.ts",
    );
    assert.deepEqual(
      events.map((event) => event.kind),
      ["w", "w", "x"],
    );
    assert.equal(
      events.every((event) => event.warn),
      true,
    );
    const contention = events.find((event) => event.kind === "x");
    assert.equal(contention?.t, T0 + 4_700);
    assert.equal(
      contention?.label,
      "write collision: store.ts (agent ↔ delegate)",
    );
  });

  it("synthesizes contention exactly at the four-second boundary", () => {
    const scene = compileScene(
      [
        ev(1_000, "w", "src/store.ts", "agent"),
        ev(5_000, "w", "src/store.ts", "delegate"),
      ],
      [agent("agent"), agent("delegate")],
    );

    assert.equal(
      scene.events.some((event) => event.kind === "x"),
      true,
    );
  });

  it("does not synthesize contention outside the four-second window", () => {
    const scene = compileScene(
      [
        ev(1_000, "w", "src/store.ts", "agent"),
        ev(5_500, "w", "src/store.ts", "delegate"),
      ],
      [agent("agent"), agent("delegate")],
    );

    assert.equal(
      scene.events.some((event) => event.kind === "x"),
      false,
    );
    assert.equal(
      scene.events.some((event) => event.warn),
      false,
    );
  });

  it("computes binary-search heat prefixes like a naive scan", () => {
    let seed = 42;
    function random() {
      seed = (seed * 1_664_525 + 1_013_904_223) % 2 ** 32;
      return seed / 2 ** 32;
    }

    const events = Array.from({ length: 120 }, (_, index) => ({
      agentId: "agent",
      t: T0 + Math.floor(random() * 60_000),
      kind: random() > 0.55 ? "w" : "r",
      path:
        index % 3 === 0
          ? "src/a.ts"
          : index % 3 === 1
            ? "src/b.ts"
            : "README.md",
    }));
    const scene = compileScene(events, [agent()]);
    const node = scene.byPath.get("src/a.ts");
    assert.notEqual(node, undefined);

    if (!node) return;
    for (let t = T0; t <= T0 + 60_000; t += 2_317) {
      const displayT = scene.timeMap.toDisplay(t);
      const fast = scene.nodeHeat(node, displayT);
      const slowTouches = node.touches.filter(
        (touch) => touch.displayT <= displayT,
      );
      const slow = slowTouches.reduce(
        (acc, touch) => {
          const dt = Math.max(0, displayT - touch.displayT);
          if (touch.kind === "r") acc.hr += Math.exp(-dt / 6_000);
          else if (
            touch.kind === "w" ||
            touch.kind === "c" ||
            touch.kind === "x"
          ) {
            acc.hw += Math.exp(-dt / 7_000);
          }
          if (touch.warn) acc.warn += Math.exp(-dt / 5_000);
          return acc;
        },
        { hr: 0, hw: 0, warn: 0 },
      );
      assert.equal(fast.cnt, slowTouches.length);
      assert.ok(
        Math.abs(fast.hr - slow.hr) < 10 ** -10,
        `expected ${fast.hr} to be close to ${slow.hr}`,
      );
      assert.ok(
        Math.abs(fast.hw - slow.hw) < 10 ** -10,
        `expected ${fast.hw} to be close to ${slow.hw}`,
      );
      assert.ok(
        Math.abs(fast.warn - slow.warn) < 10 ** -10,
        `expected ${fast.warn} to be close to ${slow.warn}`,
      );
    }

    const finalHeat = nodeHeat(node, scene.timeMap.toDisplay(T0 + 60_000));
    assert.equal(finalHeat.cnt, node.touches.length);
    assert.equal(finalHeat.last, node.touches.at(-1));
  });

  it("compresses idle gaps into display time and precomputes display timestamps", () => {
    const scene = compileScene(
      [
        ev(0, "r", "README.md"),
        ev(10_000, "w", "README.md"),
        ev(2 * 60 * 60_000 + 10_000, "r", "src/App.tsx"),
        ev(2 * 60 * 60_000 + 20_000, "w", "src/App.tsx"),
      ],
      [agent()],
    );

    assert.equal(scene.t0, T0);
    assert.equal(scene.durationMs, 2 * 60 * 60_000 + 20_000);
    assert.equal(scene.d1, 22_500);
    assert.equal(scene.timeMap.gaps.length, 1);
    assert.deepEqual(
      scene.events.map((event) => event.displayT),
      [0, 10_000, 12_500, 22_500],
    );

    for (const event of scene.events) {
      assert.equal(event.displayT, scene.timeMap.toDisplay(event.t));
    }

    for (const node of scene.nodes) {
      for (const touch of node.touches) {
        assert.equal(touch.displayT, scene.timeMap.toDisplay(touch.t));
        assert.equal(touch.event.displayT, touch.displayT);
      }
    }

    assert.equal(
      scene.agents[0].points.every((point) => Number.isFinite(point.displayT)),
      true,
    );
    assert.deepEqual(
      scene.agents[0].pointTimes,
      scene.agents[0].points.map((point) => point.displayT),
    );
  });

  it("carries scene-level agent indices and primary agent id", () => {
    const scene = compileScene(
      [ev(1_000, "r", "README.md"), ev(2_000, "w", "src/App.tsx")],
      [agent("agent")],
    );

    assert.equal(scene.primaryAgentId, "agent");
    assert.equal(scene.agentsById.get("agent"), scene.agents[0]);
    assert.deepEqual(
      scene.events.map((event) => event.id),
      ["event-0", "event-1"],
    );
    // Timeline times are display-space: scene start (real t=1_000) anchors to 0.
    assert.deepEqual(scene.agents[0].timeline.times, [0, 1_000]);
    assert.deepEqual(scene.agents[0].timeline.readPrefixCounts, [0, 1, 1]);
    assert.deepEqual(scene.agents[0].timeline.writePrefixCounts, [0, 0, 1]);
  });

  it("uses one layout object for axis strip and tree/chart geometry", () => {
    const wide = getActivityPanelLayout(640);
    assert.equal(wide.treeWidth, 252);
    assert.equal(wide.chartX0, 252);
    assert.equal(wide.chartTop, 24);
    assert.equal(wide.axisHeight, 24);

    const narrow = getActivityPanelLayout(180);
    assert.equal(narrow.treeWidth, 140);
    assert.equal(narrow.chartX0, 140);
    assert.equal(narrow.chartX1, 164);
    assert.equal(narrow.chartTop, 24);
  });

  it("chooses overflow layout height when rows cannot fit", () => {
    const layout = getActivityPanelLayout(640, 100, 10);

    assert.equal(layout.rowH, 13);
    assert.equal(layout.contentHeight, 24 + 10 * 13 + 12);
    assert.ok(layout.contentHeight > 100);
  });

  it("adds hold points before row changes so threads bend only near events", () => {
    const scene = compileScene(
      [ev(1_000, "r", "README.md"), ev(5_000, "r", "src/App.tsx")],
      [agent()],
    );
    const points = scene.agents[0].points;
    const hold = points.find((point) => point.hold);

    assert.notEqual(hold, undefined);
    // Hold points are display-space: real t=5_000 maps to 4_000 (start anchors to 0).
    assert.equal(hold?.displayT, 4_000 - scene.constants.transitionMs);
    assert.equal(hold?.node.path, "README.md");
    assert.equal(points.at(-1)?.node.path, "src/App.tsx");
  });
});
