import * as React from "react";
import {
  AlertTriangle,
  Check,
  Circle,
  Database,
  Eye,
  ListChecks,
  LoaderCircle,
  Send,
  Sparkles,
  Wrench,
  type LucideIcon,
} from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { AgentSessionTranscriptVariantProvider } from "./agentSessionTranscriptContext";
import type { TranscriptItem } from "./agentSessionTypes";
import { TranscriptActivityItem } from "./activityRenderClasses/TranscriptActivityItem";
import type { AgentTranscriptIdentityProps } from "./activityRenderClasses/types";
import {
  buildModelWorkStream,
  MODEL_WORK_PHASES,
  type ModelWorkPhase,
  type ModelWorkPhaseState,
  type ModelWorkStep,
} from "./modelWorkStream";

const PHASE_PRESENTATION: Record<
  ModelWorkPhase,
  { icon: LucideIcon; label: string }
> = {
  context: { icon: Database, label: "Input" },
  explore: { icon: Eye, label: "Explore" },
  decide: { icon: ListChecks, label: "Decide" },
  act: { icon: Wrench, label: "Act" },
  deliver: { icon: Send, label: "Deliver" },
};

export function ModelWorkStream({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  isWorking,
  items,
  profiles,
}: AgentTranscriptIdentityProps & {
  isWorking: boolean;
  items: TranscriptItem[];
  profiles?: UserProfileLookup;
}) {
  const stream = React.useMemo(
    () => buildModelWorkStream(items, { isWorking }),
    [isWorking, items],
  );

  return (
    <AgentSessionTranscriptVariantProvider value="inlineTimeline">
      <div className="min-w-0" data-testid="model-work-stream">
        <div className="border-y border-border/70 py-2">
          <div className="flex min-w-0 items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="text-2xs font-semibold text-muted-foreground">
                Current focus
              </p>
              <p
                aria-live="polite"
                className="mt-0.5 truncate text-sm font-medium text-foreground"
                title={stream.focus}
              >
                {stream.focus}
              </p>
            </div>
            <StreamStatus isWorking={isWorking} />
          </div>
          <p className="mt-1.5 text-2xs text-muted-foreground/70">
            {formatTotals(stream.totals)}
          </p>
        </div>

        <ol
          aria-label="Model information flow"
          className="mt-3 grid grid-cols-5"
        >
          {MODEL_WORK_PHASES.map((phase, index) => (
            <PhaseNode
              key={phase}
              isLast={index === MODEL_WORK_PHASES.length - 1}
              phase={phase}
              state={stream.phaseStates[phase]}
            />
          ))}
        </ol>

        <div className="mt-4 flex items-center justify-between gap-3 border-b border-border/70 pb-1.5">
          <p className="text-xs font-semibold text-foreground">Agent trace</p>
          <p className="text-2xs text-muted-foreground">
            {stream.steps.length}{" "}
            {stream.steps.length === 1 ? "event" : "events"}
          </p>
        </div>
        <div className="pt-3" data-testid="model-work-stream-steps">
          {stream.steps.map((step, index) => (
            <WorkStreamStep
              agentAvatarUrl={agentAvatarUrl}
              agentName={agentName}
              agentPubkey={agentPubkey}
              isLast={index === stream.steps.length - 1}
              key={step.id}
              profiles={profiles}
              step={step}
            />
          ))}
        </div>
      </div>
    </AgentSessionTranscriptVariantProvider>
  );
}

function PhaseNode({
  isLast,
  phase,
  state,
}: {
  isLast: boolean;
  phase: ModelWorkPhase;
  state: ModelWorkPhaseState;
}) {
  const presentation = PHASE_PRESENTATION[phase];
  const Icon = presentation.icon;

  return (
    <li
      aria-current={state === "active" ? "step" : undefined}
      className="relative flex min-w-0 flex-col items-center gap-1"
    >
      {!isLast ? (
        <span
          aria-hidden="true"
          className={cn(
            "absolute left-[calc(50%+0.75rem)] right-[calc(-50%+0.75rem)] top-3 h-px",
            state === "complete" ? "bg-primary/45" : "bg-border",
          )}
        />
      ) : null}
      <span
        className={cn(
          "relative z-10 flex h-6 w-6 items-center justify-center rounded-md border bg-background",
          state === "active"
            ? "border-primary/60 text-primary"
            : state === "complete"
              ? "border-primary/30 text-foreground"
              : "border-border text-muted-foreground/45",
        )}
      >
        {state === "active" ? (
          <LoaderCircle
            aria-hidden="true"
            className="h-3.5 w-3.5 motion-safe:animate-spin"
          />
        ) : state === "complete" ? (
          <Check aria-hidden="true" className="h-3.5 w-3.5" />
        ) : (
          <Icon aria-hidden="true" className="h-3.5 w-3.5" />
        )}
      </span>
      <span
        className={cn(
          "truncate text-3xs font-medium",
          state === "idle" ? "text-muted-foreground/45" : "text-foreground",
        )}
      >
        {presentation.label}
      </span>
    </li>
  );
}

function WorkStreamStep({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  isLast,
  profiles,
  step,
}: AgentTranscriptIdentityProps & {
  isLast: boolean;
  profiles?: UserProfileLookup;
  step: ModelWorkStep;
}) {
  const presentation = PHASE_PRESENTATION[step.phase];
  const Icon = step.status === "failed" ? AlertTriangle : presentation.icon;
  const rendersExistingActivity =
    step.item.type === "tool" || step.item.type === "lifecycle";

  return (
    <div className="grid grid-cols-[1.5rem_minmax(0,1fr)] gap-2.5">
      <div className="relative flex justify-center">
        {!isLast ? (
          <span
            aria-hidden="true"
            className="absolute bottom-0 top-6 w-px bg-border/80"
          />
        ) : null}
        <span
          className={cn(
            "relative z-10 mt-0.5 flex h-5 w-5 items-center justify-center rounded-md border bg-background",
            step.status === "failed"
              ? "border-destructive/40 text-destructive"
              : step.status === "active"
                ? "border-primary/50 text-primary"
                : "border-border text-muted-foreground",
          )}
        >
          {step.status === "active" ? (
            <LoaderCircle
              aria-hidden="true"
              className="h-3 w-3 motion-safe:animate-spin"
            />
          ) : (
            <Icon aria-hidden="true" className="h-3 w-3" />
          )}
        </span>
      </div>
      <div className={cn("min-w-0", isLast ? "pb-0" : "pb-3")}>
        <div className="mb-1 flex min-w-0 items-center gap-1.5 text-3xs font-semibold">
          <span className="uppercase text-muted-foreground/70">
            {presentation.label}
          </span>
          <Circle
            aria-hidden="true"
            className="h-1 w-1 fill-current text-muted-foreground/35"
          />
          <span
            className={cn(
              step.status === "failed"
                ? "text-destructive"
                : step.status === "active"
                  ? "text-primary"
                  : "text-muted-foreground/55",
            )}
          >
            {step.status === "active"
              ? "Live"
              : step.status === "failed"
                ? "Failed"
                : "Done"}
          </span>
        </div>

        {rendersExistingActivity ? (
          <TranscriptActivityItem
            agentAvatarUrl={agentAvatarUrl}
            agentName={agentName}
            agentPubkey={agentPubkey}
            item={step.item}
            profiles={profiles}
          />
        ) : (
          <div className="min-w-0">
            <p className="text-xs font-semibold text-foreground">
              {step.label}
            </p>
            {step.detail ? (
              <p className="mt-0.5 line-clamp-2 text-xs leading-4 text-muted-foreground">
                {step.detail}
              </p>
            ) : null}
          </div>
        )}

        <ActualTrace step={step} />

        {step.finding ? (
          <div className="mt-1.5 flex min-w-0 items-start gap-1.5 border-l-2 border-status-added/35 pl-2 text-xs leading-4 text-muted-foreground">
            <Sparkles
              aria-hidden="true"
              className="mt-0.5 h-3 w-3 shrink-0 text-status-added"
            />
            <p className="line-clamp-2 min-w-0">
              <span className="font-semibold text-foreground">
                {step.signalLabel}
              </span>
              {" · "}
              {step.finding}
            </p>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function ActualTrace({ step }: { step: ModelWorkStep }) {
  return (
    <div
      className="mt-1.5 min-w-0 border-l border-border/80 pl-2"
      data-testid="model-work-actual-trace"
    >
      <div className="flex min-w-0 items-center gap-2">
        <span className="shrink-0 text-3xs font-semibold uppercase text-muted-foreground/60">
          Call
        </span>
        <code
          className="min-w-0 truncate text-2xs font-medium text-foreground/80"
          title={step.trace.name}
        >
          {step.trace.name}
        </code>
      </div>
      {step.trace.input ? (
        <TraceValue label="Input" value={step.trace.input} />
      ) : null}
      {step.trace.output ? (
        <TraceValue label="Result" value={step.trace.output} />
      ) : null}
    </div>
  );
}

function TraceValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="mt-0.5 grid min-w-0 grid-cols-[2.5rem_minmax(0,1fr)] gap-1.5 text-2xs leading-4">
      <span className="font-medium text-muted-foreground/60">{label}</span>
      <span className="line-clamp-2 min-w-0 break-words text-muted-foreground">
        {value}
      </span>
    </div>
  );
}

function StreamStatus({ isWorking }: { isWorking: boolean }) {
  return (
    <span
      className={cn(
        "inline-flex h-6 shrink-0 items-center gap-1.5 text-2xs font-semibold",
        isWorking ? "text-primary" : "text-muted-foreground",
      )}
    >
      {isWorking ? (
        <LoaderCircle
          aria-hidden="true"
          className="h-3.5 w-3.5 motion-safe:animate-spin"
        />
      ) : (
        <Check aria-hidden="true" className="h-3.5 w-3.5" />
      )}
      {isWorking ? "Working" : "Complete"}
    </span>
  );
}

function formatTotals({
  actions,
  findings,
  inputs,
}: {
  actions: number;
  findings: number;
  inputs: number;
}) {
  return [
    `${inputs} ${inputs === 1 ? "input" : "inputs"}`,
    `${findings} ${findings === 1 ? "finding" : "findings"}`,
    `${actions} ${actions === 1 ? "action" : "actions"}`,
  ].join(" · ");
}
