import { Cpu, LoaderCircle } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Switch } from "@/shared/ui/switch";
import type { useMeshComputeState } from "../hooks/useMeshComputeState";
import {
  downloadPercent,
  formatDownloadBytes,
} from "../hooks/useMeshDownloadProgress";

export function MeshComputeShareBanner({
  communityName,
  mesh,
}: {
  communityName: string;
  mesh: ReturnType<typeof useMeshComputeState>;
}) {
  const {
    downloadProgress,
    pendingAction,
    shareSwitchChecked,
    toggle,
    setSharing,
  } = mesh;
  const busy = pendingAction !== null;
  const progress = downloadProgress ? downloadPercent(downloadProgress) : null;
  const progressDetail = downloadProgress
    ? formatDownloadBytes(downloadProgress)
    : "";

  return (
    <section
      className="overflow-hidden rounded-2xl border border-foreground/15 bg-linear-to-br from-foreground/[0.07] to-muted/10"
      data-testid="compute-share-banner"
    >
      <div className="flex min-w-0 items-start gap-3 px-4 py-4">
        <span
          aria-hidden="true"
          className={cn(
            "flex size-10 shrink-0 items-center justify-center rounded-xl border border-border/70 bg-background",
            toggle.isSharing && "border-foreground/30",
          )}
        >
          {busy ? (
            <LoaderCircle className="size-4 animate-spin motion-reduce:animate-none" />
          ) : (
            <Cpu className="size-4" />
          )}
        </span>
        <div className="min-w-0 flex-1">
          <label
            className="text-base font-semibold"
            htmlFor="compute-share-banner-toggle"
          >
            Share compute
          </label>
          <p className="mt-1 max-w-2xl text-sm leading-relaxed text-muted-foreground">
            Contribute this machine’s spare capacity so {communityName}’s agents
            have more compute available. Pause anytime.
          </p>
        </div>
        <Switch
          checked={shareSwitchChecked}
          data-testid="compute-share-banner-toggle"
          disabled={busy || (!toggle.isSharing && !mesh.modelToShare)}
          id="compute-share-banner-toggle"
          onCheckedChange={setSharing}
        />
      </div>

      <div
        aria-live="polite"
        className="border-border/60 border-t bg-background/60 px-4 py-3"
      >
        <p className="text-sm" data-testid="compute-share-banner-status">
          {statusCopy(mesh, communityName)}
        </p>
        {downloadProgress ? (
          <div className="mt-2" data-testid="mesh-download-progress">
            <div className="flex justify-between gap-3 text-xs text-muted-foreground">
              <span className="truncate">
                {downloadProgress.status === "preparing"
                  ? "Preparing"
                  : "Downloading"}{" "}
                {downloadProgress.label}
              </span>
              <span className="shrink-0">
                {progress !== null
                  ? `${progress}%`
                  : progressDetail || "Working…"}
              </span>
            </div>
            {progressDetail && progress !== null ? (
              <p className="mt-1 text-2xs text-muted-foreground">
                {progressDetail}
              </p>
            ) : null}
            <div className="mt-1.5 h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                className={cn(
                  "h-full rounded-full bg-primary transition-[width] duration-300",
                  progress === null &&
                    "w-1/4 animate-pulse motion-reduce:animate-none",
                )}
                style={
                  progress !== null ? { width: `${progress}%` } : undefined
                }
              />
            </div>
          </div>
        ) : null}
        {mesh.error ? (
          <p className="mt-1 text-xs text-destructive" role="alert">
            {mesh.error}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function statusCopy(
  mesh: ReturnType<typeof useMeshComputeState>,
  communityName: string,
): string {
  const model =
    mesh.status?.modelName ?? mesh.status?.modelId ?? mesh.modelToShare;
  if (mesh.pendingAction === "start") {
    return mesh.downloadProgress
      ? "Downloading the files needed to contribute. Your tile will appear when this machine is ready."
      : "Checking this machine and preparing shared compute. Your tile will appear when it is ready.";
  }
  if (mesh.pendingAction === "stop") return "Stopping shared compute…";
  if (mesh.toggle.isSharing)
    return `This machine is sharing compute with ${communityName}${model ? ` using ${model}` : ""}.`;
  if (mesh.toggle.isConsuming)
    return "This machine is using shared compute. Turn this on to contribute too.";
  if (mesh.modelToShare)
    return "Ready to contribute. Buzz will choose compatible settings automatically.";
  return "Checking whether this machine can contribute…";
}
