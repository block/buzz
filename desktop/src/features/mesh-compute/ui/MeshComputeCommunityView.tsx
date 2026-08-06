import { Info, RefreshCw } from "lucide-react";

import { Button } from "@/shared/ui/button";
import type { MeshSnapshot } from "@/shared/api/tauriMesh";
import {
  deriveCommunityComputeMapModel,
  type CommunityComputeSnapshotInput,
} from "../communityComputeMapModel";
import { CommunityComputeHealthScorecard } from "./CommunityComputeHealthScorecard";
import { CommunityComputeKpiRow } from "./CommunityComputeKpiRow";
import { CommunityComputeTerritoryMap } from "./CommunityComputeTerritoryMap";

export function MeshComputeCommunityView({
  communityName,
  error,
  onRefresh,
  simulated,
  snapshot,
  tutorialInference = false,
}: {
  communityName: string;
  error: string | null;
  onRefresh: () => void;
  simulated: boolean;
  snapshot: MeshSnapshot | CommunityComputeSnapshotInput | null;
  tutorialInference?: boolean;
}) {
  const model = deriveCommunityComputeMapModel(snapshot);
  const inferenceDeploymentIds = tutorialInference
    ? model.deployments
        .filter(
          (deployment, index) =>
            deployment.cellCount > 1 || index % 3 === 0 || index % 7 === 2,
        )
        .map((deployment) => deployment.id)
    : [];
  const observedAt = (snapshot as MeshSnapshot | null)?.observedAt;

  return (
    <div className="space-y-5" data-testid="community-compute-view">
      {simulated ? (
        <div
          className="flex items-center gap-2 rounded-xl border border-dashed border-foreground/30 bg-foreground/[0.035] px-3 py-2 text-xs"
          data-testid="community-compute-simulated-banner"
        >
          <Info className="size-3.5" />
          <span>
            Simulated mesh for learning and development. Live community data is
            unchanged.
          </span>
        </div>
      ) : null}

      {error && !simulated ? (
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
                : "Model mesh"}
            </h2>
            <p className="text-sm text-muted-foreground">
              {model.deployments.length === 1
                ? "This machine is the foundation of the mesh. More contributors add capacity, model choice, and resilience."
                : "Each territory is one ready model on one device. Area reflects its reported model-memory tier."}
            </p>
          </div>
          {!simulated ? (
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
          ) : null}
        </div>
        <CommunityComputeTerritoryMap
          inferenceDeploymentIds={inferenceDeploymentIds}
          model={model}
        />
        <p className="mt-2 text-2xs text-muted-foreground">
          {simulated
            ? "Deterministic tutorial snapshot"
            : observedAt
              ? `Recently reported · updated ${formatAge(observedAt)}`
              : "Recently reported by community members · presence may take up to two minutes to expire"}
        </p>
      </div>

      <div>
        <div className="mb-2">
          <h2 className="text-lg font-semibold tracking-tight">Mesh health</h2>
          <p className="text-sm text-muted-foreground">
            Current ready, warming, and idle states from community status
            reports.
          </p>
        </div>
        <CommunityComputeHealthScorecard health={model.health} />
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
