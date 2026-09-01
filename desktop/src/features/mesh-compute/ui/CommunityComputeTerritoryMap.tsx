import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  hexTerritoryBoundaryEdges,
  layoutCommunityComputeHexTerritories,
  type CommunityComputeHexCell,
} from "../communityComputeHexLayout";
import type {
  CommunityComputeDeployment,
  CommunityComputeMapModel,
} from "../communityComputeMapModel";

export type CommunityComputeTerritoryMapProps = {
  model: CommunityComputeMapModel;
  className?: string;
};

type Point = { x: number; y: number };

type ViewBox = {
  x: number;
  y: number;
  width: number;
  height: number;
};

// Deliberately overlap polygons, then trim their outside with the territory
// edge. This avoids SVG antialias seams inside one model footprint.
const FUSED_TILE_RADIUS = 1.025;
const TILE_EDGE_WIDTH = 0.1;
const SQRT_THREE = Math.sqrt(3);

/**
 * Responsive SVG territory map. Each deployment owns one stable, contiguous
 * cluster of discrete hexes; health changes never move that cluster.
 */
export function CommunityComputeTerritoryMap({
  model,
  className,
}: CommunityComputeTerritoryMapProps) {
  const deploymentById = React.useMemo(
    () =>
      new Map(
        model.deployments.map((deployment) => [deployment.id, deployment]),
      ),
    [model.deployments],
  );
  const previousLayoutRef = React.useRef<ReturnType<
    typeof layoutCommunityComputeHexTerritories
  > | null>(null);
  const layout = React.useMemo(
    () =>
      layoutCommunityComputeHexTerritories(
        model.deployments.map(({ id, cellCount }) => ({ id, cellCount })),
        { previous: previousLayoutRef.current },
      ),
    [model.deployments],
  );
  React.useEffect(() => {
    previousLayoutRef.current = layout;
  }, [layout]);
  const viewBox = computeViewBox(layout.cells);
  const [hoveredId, setHoveredId] = React.useState<string | null>(null);
  const lowerTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const hoveredDeployment = hoveredId
    ? (deploymentById.get(hoveredId) ?? null)
    : null;
  const isDenseMap = model.deployments.length >= 50;

  React.useEffect(
    () => () => {
      if (lowerTimerRef.current) clearTimeout(lowerTimerRef.current);
    },
    [],
  );

  function raiseTerritory(deploymentId: string, territoryElement: SVGGElement) {
    if (lowerTimerRef.current) clearTimeout(lowerTimerRef.current);
    // SVG paints in DOM order. Move the existing node instead of rebuilding
    // children so hover stays smooth as territories scale.
    territoryElement.parentNode?.appendChild(territoryElement);
    setHoveredId(deploymentId);
  }

  function lowerTerritory() {
    if (lowerTimerRef.current) clearTimeout(lowerTimerRef.current);
    // Let the detail card linger briefly while the territory settles.
    lowerTimerRef.current = setTimeout(() => setHoveredId(null), 280);
  }

  if (model.status === "loading") {
    return (
      <section
        aria-busy="true"
        aria-label="Community compute map"
        className={cn(
          "rounded-xl border border-border/70 bg-muted/10 p-4",
          className,
        )}
        data-testid="community-compute-map-loading"
      >
        <div className="h-48 animate-pulse rounded-lg bg-muted/50 motion-reduce:animate-none" />
        <p className="mt-3 text-sm text-muted-foreground">
          Mapping community compute…
        </p>
      </section>
    );
  }

  if (model.status === "empty" || viewBox === null) {
    return (
      <section
        aria-label="Community compute map"
        className={cn(
          "flex min-h-48 flex-col items-center justify-center rounded-xl border border-dashed border-border px-5 py-8 text-center",
          className,
        )}
        data-testid="community-compute-map-empty"
      >
        <EmptyHexes />
        <p className="mt-3 text-sm font-medium">No compute territories yet</p>
        <p className="mt-1 max-w-sm text-xs leading-relaxed text-muted-foreground">
          Shared compute appears here when members make it available to the
          community.
        </p>
      </section>
    );
  }

  return (
    <section
      aria-label="Community compute map"
      className={cn(
        "min-w-0 space-y-4 rounded-xl border border-border/70 bg-muted/10 p-3",
        className,
      )}
      data-testid="community-compute-territory-map"
    >
      <div className="relative flex min-h-72 min-w-0 flex-col items-center justify-center gap-4 overflow-hidden rounded-lg border border-border/50 bg-background/70 p-3 sm:min-h-96">
        <svg
          aria-label={`${model.deployments.length} community compute territories. Focus or hover a territory for details.`}
          className="h-auto max-h-[28rem] w-full overflow-visible"
          data-testid="community-compute-map-svg"
          preserveAspectRatio="xMidYMid meet"
          role="img"
          viewBox={`${viewBox.x} ${viewBox.y} ${viewBox.width} ${viewBox.height}`}
        >
          {layout.territories.map((territory) => {
            const deployment = deploymentById.get(territory.deploymentId);
            if (!deployment) return null;
            return (
              // SVG groups have no semantic element for hover/focus detail disclosure.
              // biome-ignore lint/a11y/noStaticElementInteractions: keyboard focus mirrors hover without making the territory an action.
              <g
                aria-label={territoryAccessibleLabel(deployment)}
                className="origin-center outline-hidden [transform-box:fill-box] transition-[opacity,transform] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] hover:scale-[var(--mesh-hover-scale)] focus-visible:scale-[var(--mesh-hover-scale)] focus-visible:opacity-90 motion-reduce:transition-none"
                data-deployment-id={deployment.id}
                data-cell-count={deployment.cellCount}
                data-testid={`community-compute-territory-${testId(deployment.id)}`}
                key={deployment.id}
                onBlur={lowerTerritory}
                onFocus={(event) =>
                  raiseTerritory(deployment.id, event.currentTarget)
                }
                onMouseEnter={(event) =>
                  raiseTerritory(deployment.id, event.currentTarget)
                }
                onMouseLeave={lowerTerritory}
                style={territoryHoverStyle(
                  territory.cells,
                  viewBox,
                  isDenseMap,
                )}
                tabIndex={0}
              >
                <title>{territoryAccessibleLabel(deployment)}</title>
                <TerritoryShape cells={territory.cells} />
              </g>
            );
          })}
        </svg>
        {hoveredDeployment ? (
          <div
            className="pointer-events-none absolute bottom-3 left-3 max-w-64 rounded-xl border border-border/70 bg-popover/95 px-3 py-2 shadow-lg backdrop-blur-sm"
            data-testid="community-compute-hover-details"
          >
            <p
              className="truncate text-xs font-semibold"
              title={hoveredDeployment.modelId}
            >
              {shortModelName(hoveredDeployment.modelId)}
            </p>
            <p className="mt-0.5 truncate text-2xs text-muted-foreground">
              {hoveredDeployment.deviceLabel} ·{" "}
              {formatCapacity(hoveredDeployment.capacityGb)}
            </p>
            <p className="mt-1 text-2xs text-muted-foreground">
              Available to the community
            </p>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function TerritoryShape({
  cells,
}: {
  cells: readonly CommunityComputeHexCell[];
}) {
  const boundary = hexTerritoryBoundaryEdges(cells);
  return (
    <g>
      {cells.map((cell) => {
        const center = axialToPoint(cell.q, cell.r);
        return (
          <polygon
            className="fill-foreground stroke-transparent"
            data-testid="community-compute-tile"
            key={cell.id}
            points={hexPoints(center, FUSED_TILE_RADIUS)}
          />
        );
      })}
      <path
        className="fill-none stroke-background"
        d={edgesToPath(boundary)}
        data-testid="community-compute-territory-boundary"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={TILE_EDGE_WIDTH}
      />
    </g>
  );
}

function territoryHoverStyle(
  cells: readonly CommunityComputeHexCell[],
  viewBox: ViewBox,
  dense: boolean,
): React.CSSProperties {
  const centers = cells.map((cell) => axialToPoint(cell.q, cell.r));
  const minX = Math.min(...centers.map(({ x }) => x)) - 1;
  const maxX = Math.max(...centers.map(({ x }) => x)) + 1;
  const minY = Math.min(...centers.map(({ y }) => y)) - 1;
  const maxY = Math.max(...centers.map(({ y }) => y)) + 1;
  const territorySpan = Math.max(maxX - minX, maxY - minY);
  const mapSpan = Math.min(viewBox.width, viewBox.height);
  // Normalize every hovered territory toward the same visual footprint. Dense
  // maps therefore expand dramatically while already-large sparse territories
  // receive only a restrained lift.
  const targetSpan = mapSpan * (dense ? 0.18 : 0.16);
  const scale = Math.max(1.06, Math.min(8, targetSpan / territorySpan));
  return { "--mesh-hover-scale": scale } as React.CSSProperties;
}

function edgesToPath(
  edges: ReturnType<typeof hexTerritoryBoundaryEdges>,
): string {
  return edges
    .map(
      (edge) =>
        `M ${edge.start.x} ${edge.start.y} L ${edge.end.x} ${edge.end.y}`,
    )
    .join(" ");
}

function EmptyHexes() {
  const centers = [
    { x: 2, y: 1.75 },
    { x: 3.58, y: 2.65 },
    { x: 2, y: 3.55 },
    { x: 0.42, y: 2.65 },
  ];
  return (
    <svg aria-hidden="true" className="h-14 w-20" viewBox="-0.7 0.5 5.4 4.3">
      {centers.map((center) => (
        <polygon
          className="fill-muted/40 stroke-muted-foreground/40"
          key={`${center.x}:${center.y}`}
          points={hexPoints(center, 0.82)}
          strokeDasharray="2 2"
          strokeWidth="0.08"
        />
      ))}
    </svg>
  );
}

function axialToPoint(q: number, r: number): Point {
  return { x: SQRT_THREE * (q + r / 2), y: 1.5 * r };
}

function hexPoints(center: Point, radius: number): string {
  return Array.from({ length: 6 }, (_, index) => {
    const angle = ((60 * index - 30) * Math.PI) / 180;
    return `${center.x + radius * Math.cos(angle)},${center.y + radius * Math.sin(angle)}`;
  }).join(" ");
}

function computeViewBox(
  cells: readonly CommunityComputeHexCell[],
): ViewBox | null {
  if (cells.length === 0) return null;
  const centers = cells.map((cell) => axialToPoint(cell.q, cell.r));
  const minX = Math.min(...centers.map(({ x }) => x)) - 1.4;
  const maxX = Math.max(...centers.map(({ x }) => x)) + 1.4;
  const minY = Math.min(...centers.map(({ y }) => y)) - 1.4;
  const maxY = Math.max(...centers.map(({ y }) => y)) + 1.4;
  return {
    x: minX,
    y: minY,
    width: Math.max(2.8, maxX - minX),
    height: Math.max(2.8, maxY - minY),
  };
}

function shortModelName(modelId: string): string {
  const tail = modelId.split("/").at(-1) ?? modelId;
  return tail.length > 34 ? `${tail.slice(0, 31)}…` : tail;
}

function territoryAccessibleLabel(
  deployment: CommunityComputeDeployment,
): string {
  const capacity = formatCapacity(deployment.capacityGb);
  return `${deployment.modelId} on ${deployment.deviceLabel}, ${capacity}`;
}

function formatCapacity(value: number | null): string {
  return value === null
    ? "Capacity not reported"
    : `${Math.round(value)} GB shared`;
}

function testId(value: string): string {
  return encodeURIComponent(value).replaceAll("%", "-");
}
