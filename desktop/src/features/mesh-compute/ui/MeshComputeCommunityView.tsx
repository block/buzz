import { RefreshCw } from "lucide-react";

import { Button } from "@/shared/ui/button";
import type { MeshSnapshot } from "@/shared/api/tauriMesh";
import {
  deriveCommunityComputeMapModel,
  type CommunityComputeSnapshotInput,
} from "../communityComputeMapModel";
import { CommunityComputeKpiRow } from "./CommunityComputeKpiRow";
import { CommunityComputeTerritoryMap } from "./CommunityComputeTerritoryMap";

export function MeshComputeCommunityView({
  communityName,
  error,
  onRefresh,
  snapshot,
}: {
  communityName: string;
  error: string | null;
  onRefresh: () => void;
  snapshot: MeshSnapshot | CommunityComputeSnapshotInput | null;
}) {
  const model = deriveCommunityComputeMapModel(snapshot);
  const observedAt = (snapshot as MeshSnapshot | null)?.observedAt;

  return (
    <div className="space-y-5" data-testid="community-compute-view">
      {error ? (
        <div className="flex items-center justify-between gap-3 rounded-xl border border-destructive/40 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          <span>
            Couldn’t load {communityName} compute: {error}
          </span>
          <Button onClick={onRefresh} size="xs" type="button" variant="outline">
            <RefreshCw /> Try again
          </Button>
        </div>
      ) : null}

      <CommunityComputeKpiRow kpis={model.kpis} />

      <div>
        <div className="mb-2 flex items-end justify-between gap-3">
          <div>
            <h2 className="text-lg font-semibold tracking-tight">
              {model.deployments.length === 1
                ? "Your community’s first compute node"
                : "Community mesh"}
            </h2>
            <p className="text-sm text-muted-foreground">
              {model.deployments.length === 1
                ? "This machine is the foundation of the mesh. More contributors add capacity, model choice, and resilience."
                : "See the compute currently available to your community. Larger territories offer more model memory."}
            </p>
          </div>
          <Button
            aria-label="Refresh community compute"
            onClick={onRefresh}
            size="icon-xs"
            title="Refresh community compute"
            type="button"
            variant="ghost"
          >
            <RefreshCw />
          </Button>
        </div>
        <CommunityComputeTerritoryMap model={model} />
        <p className="mt-2 text-2xs text-muted-foreground">
          {observedAt
            ? `Recently reported · updated ${formatAge(observedAt)}`
            : "Recently reported by community members · presence may take up to two minutes to expire"}
        </p>
      </div>
    </div>
  );
}

function formatAge(timestamp: number): string {
  const timestampMs = timestamp < 10_000_000_000 ? timestamp * 1000 : timestamp;
  const seconds = Math.max(0, Math.round((Date.now() - timestampMs) / 1000));
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  return `${Math.floor(seconds / 60)}m ago`;
}
