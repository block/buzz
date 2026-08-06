import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  hexTerritoryBoundaryEdges,
  layoutCommunityComputeHexTerritories,
  type CommunityComputeHexCell,
} from "../communityComputeHexLayout";
import type {
  CommunityComputeDeployment,
  CommunityComputeDeploymentHealth,
  CommunityComputeMapModel,
} from "../communityComputeMapModel";

export type CommunityComputeTerritoryMapProps = {
  model: CommunityComputeMapModel;
  selectedDeploymentId?: string | null;
  onSelectedDeploymentChange?: (deploymentId: string) => void;
  className?: string;
  /** Tutorial-only simulated activity; live snapshots cannot attribute requests. */
  inferenceDeploymentIds?: readonly string[];
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
  selectedDeploymentId,
  onSelectedDeploymentChange,
  className,
  inferenceDeploymentIds = [],
}: CommunityComputeTerritoryMapProps) {
  const [internalSelection, setInternalSelection] = React.useState<
    string | null
  >(null);
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
  const fallbackId = model.deployments[0]?.id ?? null;
  const requestedId = selectedDeploymentId ?? internalSelection;
  const activeId =
    requestedId !== null && deploymentById.has(requestedId)
      ? requestedId
      : fallbackId;
  const activeDeployment = activeId
    ? (deploymentById.get(activeId) ?? null)
    : null;
  const viewBox = computeViewBox(layout.cells);
  const inferenceIds = React.useMemo(
    () => new Set(inferenceDeploymentIds),
    [inferenceDeploymentIds],
  );
  const [hoveredId, setHoveredId] = React.useState<string | null>(null);
  const [deploymentSearch, setDeploymentSearch] = React.useState("");
  const hoveredDeployment = hoveredId
    ? (deploymentById.get(hoveredId) ?? null)
    : null;
  const isHeroDeployment = model.deployments.length === 1;

  function selectDeployment(deploymentId: string) {
    if (selectedDeploymentId === undefined) {
      setInternalSelection(deploymentId);
    }
    onSelectedDeploymentChange?.(deploymentId);
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
          Mapping community deployments…
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
          Deployments appear here when members share compute with the community.
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
      <div className="relative flex min-h-72 min-w-0 items-center justify-center overflow-hidden rounded-lg border border-border/50 bg-background/70 p-3 sm:min-h-96">
        <svg
          aria-label={`${model.deployments.length} community compute deployments. Select a territory for details.`}
          className="h-auto max-h-[28rem] w-full overflow-visible"
          data-testid="community-compute-map-svg"
          preserveAspectRatio="xMidYMid meet"
          role="img"
          viewBox={`${viewBox.x} ${viewBox.y} ${viewBox.width} ${viewBox.height}`}
        >
          {layout.territories.map((territory) => {
            const deployment = deploymentById.get(territory.deploymentId);
            if (!deployment) return null;
            const selected = deployment.id === activeId;
            const inferenceActive = inferenceIds.has(deployment.id);
            return (
              // SVG groups have no semantic button equivalent; keyboard selection remains in the adjacent deployment list.
              // biome-ignore lint/a11y/useSemanticElements: SVG has no native button element.
              <g
                aria-label={territoryAccessibleLabel(deployment)}
                className="cursor-pointer outline-hidden transition-opacity hover:opacity-90 focus-visible:opacity-80"
                data-deployment-id={deployment.id}
                data-health={deployment.health}
                data-cell-count={deployment.cellCount}
                data-inference-active={inferenceActive}
                data-selected={selected}
                data-testid={`community-compute-territory-${testId(deployment.id)}`}
                key={deployment.id}
                onClick={() => selectDeployment(deployment.id)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") {
                    event.preventDefault();
                    selectDeployment(deployment.id);
                  }
                }}
                onMouseEnter={() => setHoveredId(deployment.id)}
                onMouseLeave={() => setHoveredId(null)}
                role="button"
                tabIndex={0}
              >
                <title>{territoryAccessibleLabel(deployment)}</title>
                <TerritoryShape
                  cells={territory.cells}
                  inferenceActive={inferenceActive}
                />
              </g>
            );
          })}
        </svg>
        {isHeroDeployment && activeDeployment ? (
          <HeroDeploymentOverlay
            deployment={activeDeployment}
            inferenceActive={inferenceIds.has(activeDeployment.id)}
          />
        ) : null}
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
            <p className="mt-1 text-2xs capitalize text-muted-foreground">
              {inferenceIds.has(hoveredDeployment.id)
                ? "Simulated inference"
                : hoveredDeployment.health}{" "}
              · {hoveredDeployment.cellCount}{" "}
              {hoveredDeployment.cellCount === 1 ? "cell" : "cells"}
            </p>
          </div>
        ) : null}
      </div>

      <div className="grid min-w-0 gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(16rem,0.36fr)]">
        <DeploymentList
          activeId={activeId}
          deployments={model.deployments}
          onSearchChange={setDeploymentSearch}
          onSelect={selectDeployment}
          search={deploymentSearch}
        />
        {activeDeployment ? (
          <DeploymentDetails deployment={activeDeployment} />
        ) : null}
      </div>
    </section>
  );
}

function HeroDeploymentOverlay({
  deployment,
  inferenceActive,
}: {
  deployment: CommunityComputeDeployment;
  inferenceActive: boolean;
}) {
  return (
    <div
      className="pointer-events-none absolute left-1/2 top-1/2 w-[min(19rem,calc(100%-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-background/20 bg-background/90 p-3 text-center shadow-xl backdrop-blur-md"
      data-testid="community-compute-hero-deployment"
    >
      <div className="flex items-center justify-center gap-2">
        <span
          className={cn(
            "size-2 rounded-full",
            deployment.health === "healthy"
              ? "bg-emerald-500"
              : "bg-muted-foreground",
          )}
        />
        <span className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          {inferenceActive ? "Serving now" : deployment.health}
        </span>
      </div>
      <h3
        className="mt-1 truncate text-sm font-semibold"
        title={deployment.modelId}
      >
        {shortModelName(deployment.modelId)}
      </h3>
      <p className="mt-0.5 truncate text-xs text-muted-foreground">
        {deployment.deviceLabel}
        {deployment.isSelf ? " · This machine" : ""}
      </p>
      <dl className="mt-2 grid grid-cols-3 divide-x divide-border/60 rounded-lg border border-border/60 bg-background/70 py-2">
        <HeroMetric
          label="Shared memory"
          value={formatCapacity(deployment.capacityGb)}
        />
        <HeroMetric
          label="Model footprint"
          value={formatCapacity(deployment.source.modelSizeGb ?? null)}
        />
        <HeroMetric label="Availability" value="Community ready" />
      </dl>
    </div>
  );
}

function HeroMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 px-2">
      <dt className="truncate text-3xs uppercase tracking-wide text-muted-foreground">
        {label}
      </dt>
      <dd className="mt-0.5 truncate text-2xs font-semibold" title={value}>
        {value}
      </dd>
    </div>
  );
}

function TerritoryShape({
  cells,
  inferenceActive,
}: {
  cells: readonly CommunityComputeHexCell[];
  inferenceActive: boolean;
}) {
  const boundary = hexTerritoryBoundaryEdges(cells);
  const fillClass = "fill-foreground";
  const territoryAnimationStyle = inferenceActive
    ? ({
        "--mesh-breath-duration": `${5.8 + (stableVisualHash(cells[0]?.deploymentId ?? "") % 7) * 0.24}s`,
        animationDelay: `${-((stableVisualHash(`${cells[0]?.deploymentId}:phase`) % 37) / 10)}s`,
      } as React.CSSProperties)
    : undefined;
  return (
    <g>
      {cells.map((cell) => {
        const center = axialToPoint(cell.q, cell.r);
        return (
          <polygon
            className={cn(
              fillClass,
              "stroke-transparent",
              inferenceActive &&
                "animate-mesh-inference motion-reduce:animate-none",
            )}
            data-testid="community-compute-tile"
            key={cell.id}
            points={hexPoints(center, FUSED_TILE_RADIUS)}
            style={territoryAnimationStyle}
          />
        );
      })}
      <path
        className={cn(
          "fill-none stroke-background transition-[opacity,stroke-width] duration-200 motion-reduce:transition-none",
          fillClass,
        )}
        d={edgesToPath(boundary)}
        data-testid="community-compute-territory-boundary"
        strokeLinecap="round"
        strokeLinejoin="round"
        // A crisp opaque perimeter cuts an empty gutter between territories.
        strokeWidth={TILE_EDGE_WIDTH}
      />
    </g>
  );
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

function DeploymentList({
  deployments,
  activeId,
  onSearchChange,
  onSelect,
  search,
}: {
  deployments: readonly CommunityComputeDeployment[];
  activeId: string | null;
  onSearchChange: (value: string) => void;
  onSelect: (id: string) => void;
  search: string;
}) {
  const query = search.trim().toLowerCase();
  const visibleDeployments = query
    ? deployments.filter((deployment) =>
        [deployment.modelId, deployment.deviceLabel]
          .filter(Boolean)
          .some((value) => value?.toLowerCase().includes(query)),
      )
    : deployments;
  return (
    <div className="min-w-0">
      <div className="mb-2 flex items-center justify-between gap-3">
        <p className="text-xs font-medium text-muted-foreground">Deployments</p>
        <span className="text-2xs tabular-nums text-muted-foreground">
          {visibleDeployments.length} of {deployments.length}
        </span>
      </div>
      <input
        aria-label="Search deployments"
        className="mb-2 h-9 w-full rounded-lg border border-input/60 bg-background px-3 text-xs outline-hidden placeholder:text-muted-foreground focus-visible:ring-2 focus-visible:ring-ring"
        onChange={(event) => onSearchChange(event.target.value)}
        placeholder="Search models or devices"
        type="search"
        value={search}
      />
      <ul
        aria-label="Community compute deployments"
        className="max-h-72 space-y-1 overflow-y-auto pr-1"
        data-testid="community-compute-deployment-list"
      >
        {visibleDeployments.map((deployment) => {
          const selected = deployment.id === activeId;
          return (
            <li key={deployment.id}>
              <button
                aria-pressed={selected}
                className={cn(
                  "flex w-full items-center gap-2 rounded-lg border border-transparent px-2.5 py-2 text-left text-xs transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                  selected && "border-border bg-muted/70 text-foreground",
                )}
                data-testid={`community-compute-deployment-${testId(deployment.id)}`}
                onClick={() => onSelect(deployment.id)}
                type="button"
              >
                <HealthGlyph health={deployment.health} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-medium">
                    {shortModelName(deployment.modelId)}
                  </span>
                  <span className="block truncate text-2xs text-muted-foreground">
                    {deployment.deviceLabel}
                  </span>
                </span>
                <span className="shrink-0 text-2xs capitalize text-muted-foreground">
                  {deployment.health}
                </span>
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function DeploymentDetails({
  deployment,
}: {
  deployment: CommunityComputeDeployment;
}) {
  return (
    <article
      aria-live="polite"
      className="min-w-0 rounded-lg border border-border/60 bg-background/80 p-3"
      data-testid="community-compute-deployment-details"
    >
      <div className="flex items-start gap-2">
        <HealthGlyph health={deployment.health} />
        <div className="min-w-0 flex-1">
          <h3
            className="truncate text-sm font-semibold"
            title={deployment.modelId}
          >
            {shortModelName(deployment.modelId)}
          </h3>
          <p className="mt-0.5 truncate text-xs text-muted-foreground">
            {deployment.deviceLabel}
            {deployment.isSelf ? " · This machine" : ""}
          </p>
          <p className="mt-0.5 text-2xs capitalize text-muted-foreground">
            {deployment.health} · {deployment.cellCount}{" "}
            {deployment.cellCount === 1 ? "cell" : "cells"}
          </p>
        </div>
      </div>
      {deployment.healthReason ? (
        <p className="mt-2 text-xs leading-relaxed text-muted-foreground">
          {deployment.healthReason}
        </p>
      ) : null}
      <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
        <Detail
          label="Shared memory"
          value={formatCapacity(deployment.capacityGb)}
        />
        <Detail label="Model" value={deployment.modelId} />
        <Detail
          label="Reported"
          value={formatReportedAt(deployment.source.reportedAt)}
        />
        <Detail
          label="Model footprint"
          value={formatCapacity(deployment.source.modelSizeGb ?? null)}
        />
      </dl>
    </article>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <dt className="text-2xs text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 truncate font-medium" title={value}>
        {value}
      </dd>
    </div>
  );
}

function HealthGlyph({ health }: { health: CommunityComputeDeploymentHealth }) {
  const cue = healthCue(health);
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex size-4 shrink-0 items-center justify-center rounded-full border border-foreground/45 text-3xs font-bold leading-none",
        health === "warming" &&
          "animate-pulse border-dashed motion-reduce:animate-none",
        health === "idle" && "border-dotted text-muted-foreground",
        health === "degraded" && "border-dashed",
        health === "offline" &&
          "border-muted-foreground/60 text-muted-foreground",
        health === "unknown" && "border-dotted text-muted-foreground",
      )}
    >
      {cue.glyph}
    </span>
  );
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

function healthCue(health: CommunityComputeDeploymentHealth): {
  dashArray?: string;
  glyph: string;
  strokeWidth: number;
} {
  switch (health) {
    case "healthy":
      return { glyph: "✓", strokeWidth: 0.075 };
    case "warming":
      return { glyph: "↻", dashArray: "0.2 0.12", strokeWidth: 0.085 };
    case "idle":
      return { glyph: "–", dashArray: "0.04 0.14", strokeWidth: 0.08 };
    case "degraded":
      return { glyph: "!", dashArray: "0.3 0.12", strokeWidth: 0.11 };
    case "offline":
      return { glyph: "×", dashArray: "0.08 0.1", strokeWidth: 0.075 };
    case "unknown":
      return { glyph: "?", dashArray: "0.03 0.12", strokeWidth: 0.075 };
  }
}

function stableVisualHash(value: string): number {
  let hash = 2166136261;
  for (const character of value) {
    hash ^= character.codePointAt(0) ?? 0;
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function shortModelName(modelId: string): string {
  const tail = modelId.split("/").at(-1) ?? modelId;
  return tail.length > 34 ? `${tail.slice(0, 31)}…` : tail;
}

function territoryAccessibleLabel(
  deployment: CommunityComputeDeployment,
): string {
  const capacity = formatCapacity(deployment.capacityGb);
  return `${deployment.modelId} on ${deployment.deviceLabel}, ${deployment.health}, ${capacity}, ${deployment.cellCount} ${deployment.cellCount === 1 ? "cell" : "cells"}`;
}

function formatCapacity(value: number | null): string {
  return value === null
    ? "Capacity not reported"
    : `${Math.round(value)} GB shared`;
}

function formatReportedAt(value: number | null | undefined): string {
  if (value === null || value === undefined) return "Not reported";
  const milliseconds = value < 10_000_000_000 ? value * 1000 : value;
  return new Date(milliseconds).toLocaleString();
}

function testId(value: string): string {
  return encodeURIComponent(value).replaceAll("%", "-");
}
