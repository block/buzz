import assert from "node:assert/strict";
import test from "node:test";

import {
  hexCoordinateKey,
  hexDistance,
  hexNeighbours,
  hexTerritoryBoundaryEdges,
  hexTerritoryInteriorEdges,
  isContiguousHexTerritory,
  layoutCommunityComputeHexTerritories,
} from "./communityComputeHexLayout.ts";
import { LARGE_DENSE_COMMUNITY_COMPUTE_FIXTURE } from "./communityComputeFixtures.ts";
import { deriveCommunityComputeMapModel } from "./communityComputeMapModel.ts";

function assertStructuralInvariants(layout) {
  const occupied = new Set(layout.cells.map(hexCoordinateKey));
  assert.equal(occupied.size, layout.cells.length, "cells must not overlap");
  assert.ok(
    layout.territories.every((territory) =>
      isContiguousHexTerritory(territory.cells),
    ),
    "every territory must be contiguous",
  );
  if (layout.cells.length > 0) {
    const reached = new Set([hexCoordinateKey(layout.cells[0])]);
    const queue = [layout.cells[0]];
    for (let cursor = 0; cursor < queue.length; cursor += 1) {
      for (const neighbour of hexNeighbours(queue[cursor])) {
        const key = hexCoordinateKey(neighbour);
        if (occupied.has(key) && !reached.has(key)) {
          reached.add(key);
          queue.push(neighbour);
        }
      }
    }
    assert.equal(
      reached.size,
      occupied.size,
      "occupied mesh must be connected",
    );
  }
}

function organicGaps(layout) {
  const occupied = new Set(layout.cells.map(hexCoordinateKey));
  const empty = new Map();
  for (const cell of layout.cells) {
    for (const neighbour of hexNeighbours(cell)) {
      const key = hexCoordinateKey(neighbour);
      if (!occupied.has(key)) empty.set(key, neighbour);
    }
  }
  return [...empty.values()].filter(
    (cell) =>
      hexNeighbours(cell).filter((neighbour) =>
        occupied.has(hexCoordinateKey(neighbour)),
      ).length >= 2,
  );
}

function assertBalancedSparseFrontier(layout, deploymentCount) {
  const points = layout.cells.map((cell) => ({
    x: Math.sqrt(3) * (cell.q + cell.r / 2),
    y: 1.5 * cell.r,
  }));
  const width =
    Math.max(...points.map(({ x }) => x)) -
    Math.min(...points.map(({ x }) => x)) +
    Math.sqrt(3);
  const height =
    Math.max(...points.map(({ y }) => y)) -
    Math.min(...points.map(({ y }) => y)) +
    2;
  const aspectRatio = width / height;
  assert.ok(
    aspectRatio >= 0.8 && aspectRatio <= 1.3,
    `${deploymentCount} deployments have unbalanced aspect ratio ${aspectRatio}`,
  );

  const radius = Math.max(
    ...layout.cells.map((cell) => hexDistance(cell, { q: 0, r: 0 })),
  );
  const directionalRadii = Array(6).fill(0);
  for (const cell of layout.cells) {
    const projections = [
      cell.q,
      -cell.r,
      -cell.q - cell.r,
      -cell.q,
      cell.r,
      cell.q + cell.r,
    ];
    for (const [index, projection] of projections.entries()) {
      directionalRadii[index] = Math.max(directionalRadii[index], projection);
    }
  }
  assert.ok(
    Math.max(...directionalRadii) - Math.min(...directionalRadii) <= 2,
    `${deploymentCount} deployments must reach every side evenly`,
  );

  const outerOccupancy =
    layout.cells.filter((cell) => hexDistance(cell, { q: 0, r: 0 }) === radius)
      .length /
    (6 * radius);
  assert.ok(
    outerOccupancy >= 0.1 && outerOccupancy <= 0.55,
    `${deploymentCount} deployments need a sparse, notched outer edge (${outerOccupancy})`,
  );
}

function deployments(count) {
  const buckets = [1, 2, 4, 7];
  return Array.from({ length: count }, (_, index) => ({
    id: `deployment-${String(index + 1).padStart(4, "0")}`,
    cellCount: buckets[index % buckets.length],
  }));
}

test("layout is deterministic and independent of input order", () => {
  const input = deployments(80);
  const first = layoutCommunityComputeHexTerritories(input);
  const second = layoutCommunityComputeHexTerritories([...input].reverse());
  assert.deepEqual(first, second);
});

test("growing layouts stay balanced with sparse outer frontiers", () => {
  for (const count of [24, 250, 1_000]) {
    const layout = layoutCommunityComputeHexTerritories(deployments(count));
    assertStructuralInvariants(layout);
    assertBalancedSparseFrontier(layout, count);
    assert.ok(
      organicGaps(layout).length > 0,
      `${count} deployments should leave deterministic pockets or edge gaps`,
    );
  }
});

test("canonical territory order remains stable when previous placement is reused", () => {
  const input = deployments(60);
  const initial = layoutCommunityComputeHexTerritories(input);
  const next = layoutCommunityComputeHexTerritories([...input].reverse(), {
    previous: initial,
  });
  assert.deepEqual(
    next.territories.map(({ deploymentId }) => deploymentId),
    initial.territories.map(({ deploymentId }) => deploymentId),
  );
});

test("payload changes preserve positions because IDs own the slots", () => {
  const initial = deployments(40);
  const changed = initial.map((deployment) => ({
    ...deployment,
    label: `Changed ${deployment.id}`,
    health: "degraded",
  }));
  assert.deepEqual(
    layoutCommunityComputeHexTerritories(initial),
    layoutCommunityComputeHexTerritories(changed),
  );
});

test("surviving IDs keep positions as deployments are added and removed", () => {
  const initialInput = deployments(40);
  const initial = layoutCommunityComputeHexTerritories(initialInput);
  const nextInput = [
    ...initialInput.filter((_, index) => index % 4 !== 0),
    ...deployments(6).map((item, index) => ({
      ...item,
      id: `new-deployment-${index}`,
    })),
  ];
  const next = layoutCommunityComputeHexTerritories(nextInput, {
    previous: initial,
  });
  const initialCenters = new Map(
    initial.territories.map((territory) => [
      territory.deploymentId,
      territory.center,
    ]),
  );

  for (const territory of next.territories) {
    const oldCenter = initialCenters.get(territory.deploymentId);
    if (oldCenter !== undefined) {
      assert.deepEqual(territory.center, oldCenter);
    }
  }
  assert.equal(
    new Set(next.cells.map(hexCoordinateKey)).size,
    next.cells.length,
  );
});

test("every supported territory shape is contiguous", () => {
  const layout = layoutCommunityComputeHexTerritories([
    { id: "one", cellCount: 1 },
    { id: "two", cellCount: 2 },
    { id: "four", cellCount: 4 },
    { id: "seven", cellCount: 7 },
  ]);
  const expected = new Map([
    ["one", 1],
    ["two", 2],
    ["four", 4],
    ["seven", 7],
  ]);

  for (const territory of layout.territories) {
    assert.equal(territory.cells.length, expected.get(territory.deploymentId));
    assert.ok(
      isContiguousHexTerritory(territory.cells),
      `${territory.deploymentId} must be contiguous`,
    );
  }
});

test("fused boundaries omit interior edges for 1, 2, 4, and 7 cells", () => {
  const shapes = [
    [{ q: 0, r: 0 }],
    [
      { q: 0, r: 0 },
      { q: 1, r: 0 },
    ],
    [
      { q: 0, r: 0 },
      { q: 1, r: 0 },
      { q: 1, r: -1 },
      { q: 0, r: -1 },
    ],
    [{ q: 0, r: 0 }, ...hexNeighbours({ q: 0, r: 0 })],
  ];

  for (const [index, cells] of shapes.entries()) {
    const sharedPairs =
      cells.reduce(
        (total, cell) =>
          total +
          hexNeighbours(cell).filter((neighbour) =>
            cells.some(
              (candidate) =>
                hexCoordinateKey(candidate) === hexCoordinateKey(neighbour),
            ),
          ).length,
        0,
      ) / 2;
    const boundary = hexTerritoryBoundaryEdges(cells);
    assert.equal(boundary.length, 6 * cells.length - 2 * sharedPairs);

    const endpointDegree = new Map();
    for (const edge of boundary) {
      assert.notDeepEqual(edge.start, edge.end);
      for (const point of [edge.start, edge.end]) {
        const key = `${point.x.toFixed(8)},${point.y.toFixed(8)}`;
        endpointDegree.set(key, (endpointDegree.get(key) ?? 0) + 1);
      }
    }
    assert.ok(
      [...endpointDegree.values()].every((degree) => degree === 2),
      `shape ${index} boundary must form closed loops without dangling edges`,
    );
  }

  const edgeKey = (edge) =>
    [edge.start, edge.end]
      .map((point) => `${point.x.toFixed(8)},${point.y.toFixed(8)}`)
      .sort()
      .join("|");
  assert.deepEqual(
    hexTerritoryBoundaryEdges(shapes[3]).map(edgeKey).sort(),
    hexTerritoryBoundaryEdges([...shapes[3]].reverse())
      .map(edgeKey)
      .sort(),
    "edge membership must not depend on cell order",
  );
});

test("interior edges contain each shared device boundary exactly once", () => {
  const cells = [{ q: 0, r: 0 }, ...hexNeighbours({ q: 0, r: 0 })];
  const interior = hexTerritoryInteriorEdges(cells);
  const edgeKey = (edge) =>
    [edge.start, edge.end]
      .map((point) => `${point.x.toFixed(8)},${point.y.toFixed(8)}`)
      .sort()
      .join("|");

  assert.equal(interior.length, 12);
  assert.equal(new Set(interior.map(edgeKey)).size, interior.length);
  assert.deepEqual(
    interior.map(edgeKey).sort(),
    hexTerritoryInteriorEdges([...cells].reverse())
      .map(edgeKey)
      .sort(),
    "shared edge membership must not depend on cell order",
  );
  assert.deepEqual(hexTerritoryInteriorEdges([{ q: 0, r: 0 }]), []);
});

test("territories never emit duplicate occupied cells", () => {
  const layout = layoutCommunityComputeHexTerritories(deployments(250));
  const coordinates = layout.cells.map(hexCoordinateKey);
  assert.equal(new Set(coordinates).size, coordinates.length);
  assert.equal(
    new Set(layout.cells.map((cell) => cell.id)).size,
    layout.cells.length,
  );
});

test("duplicate deployment IDs are rejected rather than overlapped", () => {
  assert.throws(
    () =>
      layoutCommunityComputeHexTerritories([
        { id: "same", cellCount: 1 },
        { id: "same", cellCount: 7 },
      ]),
    /Duplicate community compute deployment ID: same/,
  );
});

test("empty layout has no fabricated bounds", () => {
  assert.deepEqual(layoutCommunityComputeHexTerritories([]), {
    territories: [],
    cells: [],
    bounds: null,
  });
});

test("500-deployment fixture lays out uniquely and contiguously", () => {
  const model = deriveCommunityComputeMapModel(
    LARGE_DENSE_COMMUNITY_COMPUTE_FIXTURE.snapshot,
  );
  const input = model.deployments.map(({ id, cellCount }) => ({
    id,
    cellCount,
  }));
  const layout = layoutCommunityComputeHexTerritories(input);

  assert.equal(layout.territories.length, model.deployments.length);
  assert.equal(
    layout.cells.length,
    model.deployments.reduce(
      (total, deployment) => total + deployment.cellCount,
      0,
    ),
  );
  assert.equal(
    new Set(layout.cells.map(hexCoordinateKey)).size,
    layout.cells.length,
  );
  assert.ok(
    layout.territories.every((territory) =>
      isContiguousHexTerritory(territory.cells),
    ),
  );
  assert.ok(layout.bounds !== null);
});
