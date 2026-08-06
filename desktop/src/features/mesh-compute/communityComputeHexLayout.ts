import type { HexCellCount } from "./communityComputeMapModel";

/** Axial coordinates for a pointy-top hex grid. */
export type HexCoordinate = {
  q: number;
  r: number;
};

/** Cartesian point in pointy-top hex geometry. */
export type HexPoint = {
  x: number;
  y: number;
};

/** One exposed side of a fused hex territory boundary. */
export type HexBoundaryEdge = {
  start: HexPoint;
  end: HexPoint;
};

export type CommunityComputeLayoutInput = {
  id: string;
  cellCount: HexCellCount;
};

export type CommunityComputeHexCell = HexCoordinate & {
  id: string;
  deploymentId: string;
  index: number;
};

export type CommunityComputeHexTerritory = {
  deploymentId: string;
  center: HexCoordinate;
  cells: readonly CommunityComputeHexCell[];
};

export type CommunityComputeHexBounds = {
  minQ: number;
  maxQ: number;
  minR: number;
  maxR: number;
};

export type CommunityComputeHexLayout = {
  territories: readonly CommunityComputeHexTerritory[];
  cells: readonly CommunityComputeHexCell[];
  bounds: CommunityComputeHexBounds | null;
};

export type CommunityComputeHexLayoutOptions = {
  /**
   * Reuse centres for IDs that are still present. Supplying the last layout
   * keeps a live map steady while deployments arrive, leave, or change size.
   */
  previous?: CommunityComputeHexLayout | null;
};

const HEX_DIRECTIONS: readonly HexCoordinate[] = [
  { q: 1, r: 0 },
  { q: 1, r: -1 },
  { q: 0, r: -1 },
  { q: -1, r: 0 },
  { q: -1, r: 1 },
  { q: 0, r: 1 },
];

/**
 * Lay deployments out as one organic, edge-connected honeycomb.
 *
 * Input order never affects the result. Stable hashes choose territory
 * orientation, while a deterministic low-discrepancy permutation spreads each
 * ring's placements around the whole current frontier. Completed inner rings
 * remain dense; only the growing outer layers carry deliberate notches. With a
 * previous layout, collision-free surviving shapes retain their exact cells;
 * new shapes take the first non-overlapping frontier position that shares an
 * edge with the occupied mesh.
 */
export function layoutCommunityComputeHexTerritories(
  deployments: readonly CommunityComputeLayoutInput[],
  options: CommunityComputeHexLayoutOptions = {},
): CommunityComputeHexLayout {
  const seenIds = new Set<string>();
  for (const deployment of deployments) {
    if (seenIds.has(deployment.id)) {
      throw new Error(
        `Duplicate community compute deployment ID: ${deployment.id}`,
      );
    }
    seenIds.add(deployment.id);
  }

  const ordered = [...deployments].sort(
    (left, right) =>
      stableHash(left.id) - stableHash(right.id) ||
      left.id.localeCompare(right.id),
  );
  // Previous cells are retained only when they remain collision-free. New
  // territories search the compact spiral for the first candidate that both
  // fits and touches the occupied frontier.
  const previousById = new Map(
    (options.previous?.territories ?? []).map((territory) => [
      territory.deploymentId,
      territory,
    ]),
  );
  const retainableIds = connectedRetainedTerritoryIds(ordered, previousById);
  const occupied = new Set<string>();
  const territories: CommunityComputeHexTerritory[] = [];
  const placementOrder = [...ordered].sort(
    (left, right) =>
      Number(retainableIds.has(right.id)) -
        Number(retainableIds.has(left.id)) ||
      ordered.indexOf(left) - ordered.indexOf(right),
  );

  for (const deployment of placementOrder) {
    const previous = retainableIds.has(deployment.id)
      ? previousById.get(deployment.id)
      : undefined;
    const retained =
      previous?.cells.length === deployment.cellCount
        ? previous.cells.map(({ q, r }) => ({ q, r }))
        : null;
    const retainedFits = retained?.every(
      (cell) => !occupied.has(hexCoordinateKey(cell)),
    );
    const orientation = stableHash(`${deployment.id}:orientation`) % 6;
    let center = retainedFits ? previous?.center : undefined;
    let coordinates = retainedFits ? retained : null;

    if (!coordinates || !center) {
      for (let slot = 0; ; slot += 1) {
        const candidate = balancedSpiralCoordinate(slot);
        const offsets = territoryOffsets(deployment.cellCount, orientation);
        const candidateCells = offsets.map((offset) => ({
          q: candidate.q + offset.q,
          r: candidate.r + offset.r,
        }));
        const fits = candidateCells.every(
          (cell) => !occupied.has(hexCoordinateKey(cell)),
        );
        const touches =
          occupied.size === 0 ||
          candidateCells.some((cell) =>
            hexNeighbours(cell).some((neighbour) =>
              occupied.has(hexCoordinateKey(neighbour)),
            ),
          );
        if (fits && touches) {
          center = candidate;
          coordinates = candidateCells;
          break;
        }
      }
    }

    const cells = coordinates.map(({ q, r }, index) => {
      occupied.add(hexCoordinateKey({ q, r }));
      return {
        id: `${deployment.id}@${q},${r}`,
        deploymentId: deployment.id,
        index,
        q,
        r,
      };
    });
    territories.push({ deploymentId: deployment.id, center, cells });
  }
  territories.sort(
    (left, right) =>
      ordered.findIndex((item) => item.id === left.deploymentId) -
      ordered.findIndex((item) => item.id === right.deploymentId),
  );
  const cells = territories.flatMap((territory) => territory.cells);

  return { territories, cells, bounds: findBounds(cells) };
}

/** The six axial neighbours of a cell, in stable clockwise order. */
export function hexNeighbours(coordinate: HexCoordinate): HexCoordinate[] {
  return HEX_DIRECTIONS.map((direction) => ({
    q: coordinate.q + direction.q,
    r: coordinate.r + direction.r,
  }));
}

/**
 * Return only the exposed unit-hex sides of a territory.
 *
 * Coordinates use pointy-top axial geometry. Shared sides are omitted, so the
 * result can be stroked as one fused territory rather than as overlapping
 * individual hexagons. Edge direction is stable and follows each cell's
 * perimeter; callers may chain matching endpoints when an ordered path is
 * required.
 */
export function hexTerritoryBoundaryEdges(
  cells: readonly HexCoordinate[],
): HexBoundaryEdge[] {
  const occupied = new Set(cells.map(hexCoordinateKey));
  const edges: HexBoundaryEdge[] = [];

  for (const cell of cells) {
    for (let direction = 0; direction < HEX_DIRECTIONS.length; direction += 1) {
      const neighbour = {
        q: cell.q + HEX_DIRECTIONS[direction].q,
        r: cell.r + HEX_DIRECTIONS[direction].r,
      };
      if (!occupied.has(hexCoordinateKey(neighbour))) {
        edges.push(hexEdge(cell, direction));
      }
    }
  }

  return edges;
}

/**
 * Return each side shared by two cells in a territory exactly once.
 *
 * Fused model territories do not render these edges. Split deployments use
 * them to show the device boundaries without drawing inset outlines around
 * every cell or accidentally doubling a shared stroke.
 */
export function hexTerritoryInteriorEdges(
  cells: readonly HexCoordinate[],
): HexBoundaryEdge[] {
  const occupied = new Set(cells.map(hexCoordinateKey));
  const edges: HexBoundaryEdge[] = [];

  for (const cell of cells) {
    const cellKey = hexCoordinateKey(cell);
    for (let direction = 0; direction < HEX_DIRECTIONS.length; direction += 1) {
      const neighbour = {
        q: cell.q + HEX_DIRECTIONS[direction].q,
        r: cell.r + HEX_DIRECTIONS[direction].r,
      };
      const neighbourKey = hexCoordinateKey(neighbour);
      if (occupied.has(neighbourKey) && cellKey < neighbourKey) {
        edges.push(hexEdge(cell, direction));
      }
    }
  }

  return edges;
}

function hexEdge(cell: HexCoordinate, direction: number): HexBoundaryEdge {
  const center = {
    x: Math.sqrt(3) * (cell.q + cell.r / 2),
    y: 1.5 * cell.r,
  };
  const corners = Array.from({ length: 6 }, (_, index) => {
    const angle = ((-30 + index * 60) * Math.PI) / 180;
    return {
      x: normalizeHexPointComponent(center.x + Math.cos(angle)),
      y: normalizeHexPointComponent(center.y + Math.sin(angle)),
    };
  });
  return {
    start: corners[(6 - direction) % 6],
    end: corners[(7 - direction) % 6],
  };
}

function normalizeHexPointComponent(value: number): number {
  return Math.abs(value) < Number.EPSILON * 8 ? 0 : value;
}

/** Axial hex distance, useful to consumers for borders and transitions. */
export function hexDistance(left: HexCoordinate, right: HexCoordinate): number {
  const dq = left.q - right.q;
  const dr = left.r - right.r;
  return Math.max(Math.abs(dq), Math.abs(dr), Math.abs(dq + dr));
}

/** True when every cell can be reached from every other through shared edges. */
export function isContiguousHexTerritory(
  cells: readonly HexCoordinate[],
): boolean {
  if (cells.length === 0) {
    return false;
  }
  const remaining = new Set(cells.map(hexCoordinateKey));
  const queue: HexCoordinate[] = [cells[0]];
  remaining.delete(hexCoordinateKey(cells[0]));
  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    for (const neighbour of hexNeighbours(queue[cursor])) {
      const key = hexCoordinateKey(neighbour);
      if (remaining.delete(key)) {
        queue.push(neighbour);
      }
    }
  }
  return remaining.size === 0;
}

export function hexCoordinateKey(coordinate: HexCoordinate): string {
  return `${coordinate.q},${coordinate.r}`;
}

/** Keep only the edge-connected component of valid surviving territories. */
function connectedRetainedTerritoryIds(
  ordered: readonly CommunityComputeLayoutInput[],
  previousById: ReadonlyMap<string, CommunityComputeHexTerritory>,
): Set<string> {
  const valid = ordered
    .map((deployment) => previousById.get(deployment.id))
    .filter(
      (territory): territory is CommunityComputeHexTerritory =>
        territory !== undefined &&
        territory.cells.length ===
          ordered.find((item) => item.id === territory.deploymentId)?.cellCount,
    );
  if (valid.length === 0) return new Set();

  const owner = new Map<string, string>();
  for (const territory of valid) {
    for (const cell of territory.cells) {
      const key = hexCoordinateKey(cell);
      if (owner.has(key)) return new Set();
      owner.set(key, territory.deploymentId);
    }
  }
  const retained = new Set([valid[0].deploymentId]);
  const queue = [valid[0].deploymentId];
  const byId = new Map(
    valid.map((territory) => [territory.deploymentId, territory]),
  );
  for (let cursor = 0; cursor < queue.length; cursor += 1) {
    const territory = byId.get(queue[cursor]);
    if (!territory) continue;
    for (const cell of territory.cells) {
      for (const neighbour of hexNeighbours(cell)) {
        const neighbourOwner = owner.get(hexCoordinateKey(neighbour));
        if (neighbourOwner && !retained.has(neighbourOwner)) {
          retained.add(neighbourOwner);
          queue.push(neighbourOwner);
        }
      }
    }
  }
  return retained;
}

function territoryOffsets(
  count: HexCellCount,
  orientation: number,
): HexCoordinate[] {
  const center = { q: 0, r: 0 };
  if (count === 1) {
    return [center];
  }
  if (count === 2) {
    return [center, HEX_DIRECTIONS[orientation]];
  }
  if (count === 4) {
    return [
      center,
      HEX_DIRECTIONS[orientation],
      HEX_DIRECTIONS[(orientation + 1) % 6],
      HEX_DIRECTIONS[(orientation + 2) % 6],
    ];
  }
  return [center, ...HEX_DIRECTIONS];
}

/**
 * Zero-based spiral slot: origin, then complete radius-one, radius-two, … rings.
 * Runtime is O(radius), which is O(sqrt(n)); even at dense fixture sizes the
 * largest radius remains small.
 */
function balancedSpiralCoordinate(index: number): HexCoordinate {
  if (index === 0) {
    return { q: 0, r: 0 };
  }

  let radius = 1;
  let ringStart = 1;
  while (index >= ringStart + 6 * radius) {
    ringStart += 6 * radius;
    radius += 1;
  }

  const ringLength = 6 * radius;
  const ringIndex = index - ringStart;
  // Multiplication by a number coprime with the ring length is a permutation,
  // not sampling: every slot is eventually visited. A step near half a ring
  // alternates opposite sides, preventing a partially filled layer from
  // producing the one-sided growth of a clockwise spiral.
  let step = Math.max(1, Math.floor(ringLength / 2) - 1);
  while (greatestCommonDivisor(step, ringLength) !== 1) step -= 1;
  const offsetSeed = stableHash(`ring:${radius}`) % ringLength;
  let offset = (offsetSeed + ringIndex * step) % ringLength;
  let coordinate = { q: -radius, r: radius };
  for (const direction of HEX_DIRECTIONS) {
    const steps = Math.min(radius, offset);
    coordinate = {
      q: coordinate.q + direction.q * steps,
      r: coordinate.r + direction.r * steps,
    };
    offset -= steps;
    if (offset === 0) {
      break;
    }
  }
  return coordinate;
}

function greatestCommonDivisor(left: number, right: number): number {
  let a = left;
  let b = right;
  while (b !== 0) {
    const remainder = a % b;
    a = b;
    b = remainder;
  }
  return a;
}

function findBounds(
  cells: readonly HexCoordinate[],
): CommunityComputeHexBounds | null {
  if (cells.length === 0) {
    return null;
  }
  let minQ = cells[0].q;
  let maxQ = cells[0].q;
  let minR = cells[0].r;
  let maxR = cells[0].r;
  for (const cell of cells.slice(1)) {
    minQ = Math.min(minQ, cell.q);
    maxQ = Math.max(maxQ, cell.q);
    minR = Math.min(minR, cell.r);
    maxR = Math.max(maxR, cell.r);
  }
  return { minQ, maxQ, minR, maxR };
}

/** FNV-1a over UTF-16 code units; deterministic on every JS runtime. */
function stableHash(value: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return hash >>> 0;
}
