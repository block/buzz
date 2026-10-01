import assert from "node:assert/strict";
import test from "node:test";

import { BrainPhysicsSimulation, DEFAULT_PHYSICS_CONFIG } from "./physics.ts";
import { getNodeVisualMeta, OBSIDIAN_NODE_PALETTE } from "./colors.ts";

test("BrainPhysicsSimulation: initializes with default config", () => {
  const sim = new BrainPhysicsSimulation(800, 600);
  assert.ok(sim);
  assert.equal(DEFAULT_PHYSICS_CONFIG.chargeRepulsion, 380);
});

test("BrainPhysicsSimulation: repels two overlapping nodes apart", () => {
  const sim = new BrainPhysicsSimulation(800, 600);
  const n1 = {
    id: "1",
    name: "A",
    entity_type: "project",
    description: "",
    created_at: "",
    x: 400,
    y: 300,
    vx: 0,
    vy: 0,
    radius: 10,
    color: "#fff",
    glowColor: "#fff",
  };
  const n2 = {
    id: "2",
    name: "B",
    entity_type: "file",
    description: "",
    created_at: "",
    x: 405,
    y: 300,
    vx: 0,
    vy: 0,
    radius: 10,
    color: "#fff",
    glowColor: "#fff",
  };

  sim.setGraph([n1, n2], []);
  sim.tick();

  // n1 should be pushed left (lower x), n2 pushed right (higher x)
  assert.ok(n1.x < 400, `Expected n1.x < 400, got ${n1.x}`);
  assert.ok(n2.x > 405, `Expected n2.x > 405, got ${n2.x}`);
});

test("BrainPhysicsSimulation: pulls connected nodes together via link spring", () => {
  const sim = new BrainPhysicsSimulation(800, 600, { centerGravity: 0 });
  const n1 = {
    id: "1",
    name: "A",
    entity_type: "project",
    description: "",
    created_at: "",
    x: 100,
    y: 300,
    vx: 0,
    vy: 0,
    radius: 10,
    color: "#fff",
    glowColor: "#fff",
  };
  const n2 = {
    id: "2",
    name: "B",
    entity_type: "file",
    description: "",
    created_at: "",
    x: 700,
    y: 300,
    vx: 0,
    vy: 0,
    radius: 10,
    color: "#fff",
    glowColor: "#fff",
  };
  const link = {
    id: "l1",
    source: "1",
    target: "2",
    relation_type: "depends_on",
    confidence: 1,
    valid_at: "",
    is_active: true,
    sourceNode: n1,
    targetNode: n2,
  };

  sim.setGraph([n1, n2], [link]);
  sim.tick();

  // Link distance is 110, but nodes are 600px apart, so spring should pull them closer
  assert.ok(n1.vx > 0, `Expected n1.vx > 0, got ${n1.vx}`);
  assert.ok(n2.vx < 0, `Expected n2.vx < 0, got ${n2.vx}`);
});

test("getNodeVisualMeta: returns correct Obsidian palette tokens for entities", () => {
  const project = getNodeVisualMeta("project");
  assert.equal(project.color, OBSIDIAN_NODE_PALETTE.project.color);
  assert.equal(project.label, "Project");

  const agent = getNodeVisualMeta("agent");
  assert.equal(agent.color, OBSIDIAN_NODE_PALETTE.agent.color);

  const decision = getNodeVisualMeta("decision");
  assert.equal(decision.color, OBSIDIAN_NODE_PALETTE.decision.color);

  const fallback = getNodeVisualMeta("unknown-custom-type");
  assert.equal(fallback.color, OBSIDIAN_NODE_PALETTE.default.color);
});
