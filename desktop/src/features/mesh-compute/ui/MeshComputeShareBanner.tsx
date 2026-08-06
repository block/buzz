import { Cpu, LoaderCircle, ShieldCheck, Sparkles, Zap } from "lucide-react";

import { Switch } from "@/shared/ui/switch";
import { cn } from "@/shared/lib/cn";
import type { useMeshComputeState } from "../hooks/useMeshComputeState";

export function MeshComputeShareBanner({
  communityName,
  mesh,
}: {
  communityName: string;
  mesh: ReturnType<typeof useMeshComputeState>;
}) {
  const {
    error,
    memory,
    modelToShare,
    pendingAction,
    shareSwitchChecked,
    status,
    toggle,
    setSharing,
  } = mesh;
  const busy = pendingAction !== null;
  const model = status?.modelName ?? status?.modelId ?? modelToShare;
  const capacity = memory.totalGb == null ? null : formatGb(memory.totalGb);
  const statusCopy = shareStatusCopy({
    communityName,
    modelToShare,
    pendingAction,
    statusModel: status?.modelName ?? status?.modelId ?? null,
    isConsuming: toggle.isConsuming,
    isSharing: toggle.isSharing,
  });

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
            Automatic contribution
          </label>
          <p className="mt-1 max-w-2xl text-sm leading-relaxed text-muted-foreground">
            Let Buzz choose the most useful compatible model and make this
            machine available when {communityName} needs it.
          </p>
        </div>
        <Switch
          checked={shareSwitchChecked}
          data-testid="compute-share-banner-toggle"
          disabled={busy || (!toggle.isSharing && !modelToShare)}
          id="compute-share-banner-toggle"
          onCheckedChange={setSharing}
        />
      </div>

      <div className="grid gap-px border-border/60 border-t bg-border/60 sm:grid-cols-3">
        <Benefit
          icon={Sparkles}
          label="Unlock community models"
          detail={
            model ? `Recommended: ${model}` : "Hardware-aware model selection"
          }
        />
        <Benefit
          icon={Zap}
          label="Increase availability"
          detail={
            capacity
              ? `Offer up to ${capacity}`
              : "Uses compatible spare capacity"
          }
        />
        <Benefit
          icon={ShieldCheck}
          label="You stay in control"
          detail="Pause anytime; advanced limits in Settings"
        />
      </div>

      <div className="border-border/60 border-t bg-background/60 px-4 py-2.5">
        <p
          className="text-xs text-muted-foreground"
          data-testid="compute-share-banner-status"
        >
          {statusCopy}
        </p>
        {error ? (
          <p className="mt-1 text-xs text-destructive" role="alert">
            {error}
          </p>
        ) : null}
      </div>
    </section>
  );
}

function Benefit({
  icon: Icon,
  label,
  detail,
}: {
  icon: typeof Sparkles;
  label: string;
  detail: string;
}) {
  return (
    <div className="flex items-start gap-2 bg-background/80 px-4 py-3">
      <Icon className="mt-0.5 size-3.5 shrink-0" />
      <div className="min-w-0">
        <p className="text-xs font-medium">{label}</p>
        <p
          className="mt-0.5 truncate text-2xs text-muted-foreground"
          title={detail}
        >
          {detail}
        </p>
      </div>
    </div>
  );
}

function shareStatusCopy({
  communityName,
  isConsuming,
  isSharing,
  modelToShare,
  pendingAction,
  statusModel,
}: {
  communityName: string;
  isConsuming: boolean;
  isSharing: boolean;
  modelToShare: string | null;
  pendingAction: "start" | "stop" | null;
  statusModel: string | null;
}): string {
  if (pendingAction === "start")
    return `Preparing ${statusModel ?? modelToShare ?? "a model"} for ${communityName}…`;
  if (pendingAction === "stop") return "Stopping shared compute…";
  if (isSharing)
    return `${statusModel ?? modelToShare ?? "A model"} is available to ${communityName}`;
  if (isConsuming)
    return "Using another member’s compute; enable this to contribute instead";
  if (modelToShare)
    return "Ready to contribute automatically when you turn this on";
  return "Checking this machine and choosing a compatible model…";
}

function formatGb(value: number): string {
  return `${new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 }).format(value)} GB`;
}
