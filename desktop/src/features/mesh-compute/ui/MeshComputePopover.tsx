import type * as React from "react";

import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { POPOVER_SHADOW_STYLE } from "@/shared/ui/popoverSurface";
import { Switch } from "@/shared/ui/switch";
import { requestOpenCreateAgent } from "@/features/agents/openCreateAgentEvent";

import type { useMeshComputeState } from "../hooks/useMeshComputeState";
import {
  describeStartupHeadline,
  describeStartupStage,
} from "../meshCardModel";
import { deriveMeshDetailModel } from "../meshDetailModel";
import { deriveMeshFieldModel } from "../meshFieldNodes";
import { MeshMemoryBulbs } from "./MeshMemoryBulbs";
import { MeshRadarField } from "./MeshRadarField";

/**
 * The single mesh detail surface, anchored to the sidebar Compute row.
 *
 * Consolidates what used to be a footer card plus a separate popover. Sidebar
 * real estate is premium, so the row carries one dot and everything else lives
 * here — including Stop sharing, which is why the row hides its switch once
 * sharing is on.
 *
 * Every figure is one this machine can vouch for. Peers come from live gossip,
 * so a node shown is a node we are connected to. Inbound work has no counter in
 * mesh-llm, so it is inferred by elimination and only named when that inference
 * holds. See `meshDetailModel.ts` and `meshActivity.ts`.
 */
export function MeshComputePopover({
  children,
  mesh,
  onOpenComputeSettings,
}: {
  children: React.ReactNode;
  mesh: ReturnType<typeof useMeshComputeState>;
  onOpenComputeSettings?: () => void;
}) {
  const {
    view,
    snapshot,
    status,
    usage,
    memory,
    toggle,
    row,
    callToAction,
    inboundWork,
    downloadProgress,
    pendingAction,
    shareSwitchChecked,
    modelToShare,
    setSharing,
    error,
  } = mesh;

  const model = deriveMeshDetailModel({
    view,
    snapshot,
    usage,
    isSharing: toggle.isSharing,
    inboundWork,
  });
  const isStarting = row.tone === "starting";
  const selectedModel = status?.modelName ?? status?.modelId ?? modelToShare;
  // The field renders from live gossip when a node is up, and from other
  // members' relay status notes when it is not — the community's shared compute
  // is knowable either way. See meshFieldNodes.ts.
  const field = deriveMeshFieldModel({ view, snapshot });

  return (
    <Popover>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent
        align="start"
        className="w-72"
        data-testid="mesh-compute-popover"
        side="right"
        sideOffset={8}
        style={POPOVER_SHADOW_STYLE}
      >
        <div className="flex flex-col gap-3">
          <div className="flex items-start justify-between gap-2">
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-semibold leading-tight">
                {model.capacityLabel ?? "MeshLLM"}
              </p>
              <p
                className="mt-0.5 text-2xs text-muted-foreground"
                data-testid="mesh-detail-participation"
              >
                {model.participationLabel}
              </p>
            </div>
            {/*
              The only share control. It used to be duplicated as a 70%-scaled
              unlabelled Switch on the row itself, which was hard to see and
              ambiguous about what it would do; the row now carries state and
              capacity only, and every control lives here where there is room to
              label it.
            */}
            <Switch
              aria-label={
                toggle.isSharing
                  ? "Stop sharing this computer's compute"
                  : "Share this computer's compute"
              }
              // Shared with the row switch: these are two controls for one
              // action and must not disagree during a multi-minute model load.
              checked={shareSwitchChecked}
              className="mt-0.5 shrink-0"
              data-testid="mesh-popover-share-toggle"
              disabled={pendingAction !== null}
              onCheckedChange={setSharing}
            />
          </div>

          {field.source === "none" && field.ghostCount === 0 ? null : (
            <div className="flex flex-col gap-1">
              <MeshRadarField
                busyNow={model.busyNow}
                ghostCount={field.ghostCount}
                nodes={field.nodes}
                selfCapacityGb={field.selfCapacityGb}
                selfParticipating={field.selfParticipating}
              />
              {field.caption ? (
                <p
                  className="text-2xs leading-snug text-muted-foreground"
                  data-testid="mesh-field-caption"
                >
                  {field.caption}
                </p>
              ) : null}
            </div>
          )}

          {toggle.isSharing && !isStarting ? (
            <MeshMemoryBulbs memory={memory} />
          ) : null}

          {isStarting ? (
            <div
              className="rounded-lg bg-muted/40 px-2.5 py-2"
              data-testid="mesh-detail-startup-status"
            >
              <p className="text-xs font-medium">
                {describeStartupHeadline(downloadProgress)}
              </p>
              <p className="mt-0.5 text-2xs leading-snug text-muted-foreground">
                {describeStartupStage(downloadProgress, selectedModel)}
              </p>
            </div>
          ) : null}

          <div className="flex flex-col gap-1">
            {model.activityLabel ? (
              <div className="flex items-center gap-1.5">
                {model.busyNow ? (
                  <span className="relative flex h-1.5 w-1.5">
                    <span className="absolute h-1.5 w-1.5 animate-ping rounded-full bg-emerald-500 motion-reduce:animate-none dark:bg-emerald-400" />
                    <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 dark:bg-emerald-400" />
                  </span>
                ) : null}
                <p
                  className="text-2xs text-muted-foreground"
                  data-testid="mesh-detail-activity"
                >
                  {model.activityLabel}
                </p>
              </div>
            ) : null}

            {model.originLabel ? (
              <p
                className="text-2xs text-muted-foreground"
                data-testid="mesh-detail-origin"
              >
                {model.originLabel}
              </p>
            ) : null}

            {model.modelCount > 0 ? (
              <p className="text-2xs text-muted-foreground">
                {model.modelCount === 1
                  ? "1 model ready"
                  : `${model.modelCount} models ready`}
              </p>
            ) : null}
          </div>

          {field.source === "none" ? (
            <p className="border-border/60 border-t pt-2 text-2xs leading-snug text-muted-foreground">
              No one is sharing compute here yet. Turn on sharing to start the
              community mesh.
            </p>
          ) : null}

          {callToAction.kind === "createAgent" ? (
            // A tip, not a warning: there is compute here and nothing set up to
            // use it. Disappears the moment a mesh agent exists.
            <button
              className="flex w-full items-center gap-1.5 rounded-md border border-border/60 bg-background/60 px-2 py-1.5 text-left text-2xs font-medium text-foreground/80 transition-colors hover:border-border hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              data-testid="mesh-card-create-agent"
              onClick={() => requestOpenCreateAgent()}
              type="button"
            >
              <span className="relative flex h-1.5 w-1.5 shrink-0">
                <span className="absolute h-1.5 w-1.5 animate-ping rounded-full bg-amber-500 motion-reduce:animate-none dark:bg-amber-400" />
                <span className="h-1.5 w-1.5 rounded-full bg-amber-500 dark:bg-amber-400" />
              </span>
              <span className="min-w-0 truncate">{callToAction.label}</span>
            </button>
          ) : null}

          {error ? (
            <p
              className="line-clamp-2 text-2xs leading-snug text-destructive"
              data-testid="mesh-card-error"
            >
              {error}
            </p>
          ) : null}

          {onOpenComputeSettings ? (
            <button
              className="-mb-1 self-start text-2xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={onOpenComputeSettings}
              type="button"
            >
              Advanced settings
            </button>
          ) : null}
        </div>
      </PopoverContent>
    </Popover>
  );
}
