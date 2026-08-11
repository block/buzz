import {
  ChevronLeft,
  ChevronRight,
  Compass,
  type LucideIcon,
  Radar,
  Route,
  Sparkles,
} from "lucide-react";
import * as React from "react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { TranscriptActivityItem } from "./activityRenderClasses/TranscriptActivityItem";
import type { AgentTranscriptIdentityProps } from "./activityRenderClasses/types";
import { AgentSessionTranscriptVariantProvider } from "./agentSessionTranscriptContext";
import type { TranscriptItem } from "./agentSessionTypes";
import {
  buildModelWorkStream,
  type ModelWorkMode,
  type ModelWorkStep,
} from "./modelWorkStream";

type ViewIdentityProps = AgentTranscriptIdentityProps & {
  profiles?: UserProfileLookup;
};

const MODE_PRESENTATION: Record<
  ModelWorkMode,
  { action: string; icon: LucideIcon; label: string }
> = {
  radar: { action: "observe", icon: Radar, label: "Radar" },
  explore: { action: "resolve", icon: Compass, label: "Explore" },
  steer: { action: "act", icon: Route, label: "Steer" },
};

export function ModelWorkStream({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  isWorking,
  items,
  profiles,
}: ViewIdentityProps & {
  isWorking: boolean;
  items: TranscriptItem[];
}) {
  const stream = React.useMemo(
    () => buildModelWorkStream(items, { isWorking }),
    [isWorking, items],
  );
  const [selectedStepId, setSelectedStepId] = React.useState<string | null>(
    null,
  );
  const [isFollowingLatest, setIsFollowingLatest] = React.useState(true);
  const selectedIndex = React.useMemo(() => {
    const requestedIndex = selectedStepId
      ? stream.steps.findIndex((step) => step.id === selectedStepId)
      : -1;
    return requestedIndex >= 0
      ? requestedIndex
      : Math.max(0, stream.steps.length - 1);
  }, [selectedStepId, stream.steps]);
  const selectedStep = stream.steps[selectedIndex] ?? null;

  React.useEffect(() => {
    const latestStep = stream.steps.at(-1);
    if (!latestStep) {
      setSelectedStepId(null);
      return;
    }
    if (
      isFollowingLatest ||
      !selectedStepId ||
      !stream.steps.some((step) => step.id === selectedStepId)
    ) {
      setSelectedStepId(latestStep.id);
    }
  }, [isFollowingLatest, selectedStepId, stream.steps]);

  const selectStep = React.useCallback(
    (id: string) => {
      setSelectedStepId(id);
      setIsFollowingLatest(id === stream.steps.at(-1)?.id);
    },
    [stream.steps],
  );

  const visibleSteps = React.useMemo(
    () => stream.steps.slice(0, selectedIndex + 1),
    [selectedIndex, stream.steps],
  );
  const aggregate = React.useMemo(
    () => buildHarnessAggregate(visibleSteps),
    [visibleSteps],
  );
  const identity = {
    agentAvatarUrl,
    agentName,
    agentPubkey,
    profiles,
  };

  if (!selectedStep) {
    return (
      <p className="border-y border-border/70 py-3 text-xs text-muted-foreground">
        Waiting for the first runtime event.
      </p>
    );
  }

  return (
    <AgentSessionTranscriptVariantProvider value="inlineTimeline">
      <div className="min-w-0" data-testid="model-work-stream">
        <RunSequence
          onSelect={selectStep}
          selectedIndex={selectedIndex}
          steps={stream.steps}
        />
        <CurrentEventHeader
          index={selectedIndex}
          step={selectedStep}
          total={stream.steps.length}
        />
        <div className="grid min-w-0 gap-4 border-b border-border/70 py-3 md:grid-cols-[minmax(0,1.45fr)_minmax(13rem,0.75fr)]">
          <section className="min-w-0" aria-label="Model mode routing">
            <SectionHeader
              detail="Chooses again for every event"
              title="Model mode router"
            />
            <ModeRouter mode={selectedStep.mode} />
            <div className="border-b border-border/60 py-2.5">
              <p className="text-3xs font-semibold uppercase text-muted-foreground/60">
                Why this mode
              </p>
              <p className="mt-1 text-xs leading-4 text-foreground">
                {selectedStep.modeReason}
              </p>
            </div>
            <ModeEnvelope step={selectedStep} />
            <div className="pt-3">
              <SectionHeader
                detail={selectedStep.status}
                title="Current operation"
              />
              <div className="pt-2">
                <CurrentOperation {...identity} step={selectedStep} />
              </div>
            </div>
          </section>
          <HarnessAggregate aggregate={aggregate} />
        </div>
        <CommunicationLog steps={visibleSteps.slice(-4)} />
      </div>
    </AgentSessionTranscriptVariantProvider>
  );
}

function RunSequence({
  onSelect,
  selectedIndex,
  steps,
}: {
  onSelect: (id: string) => void;
  selectedIndex: number;
  steps: ModelWorkStep[];
}) {
  const previous = steps[selectedIndex - 1];
  const next = steps[selectedIndex + 1];

  return (
    <section className="border-y border-border/70 bg-muted/10 py-2.5">
      <div className="mb-2 flex items-center justify-between gap-3">
        <p className="text-xs font-semibold text-foreground">
          Event sequence
          <span className="ml-1.5 font-normal text-muted-foreground">
            {selectedIndex + 1} of {steps.length}
          </span>
        </p>
        <div className="flex items-center gap-1">
          <button
            aria-label="Previous event"
            className="inline-flex h-6 w-6 items-center justify-center rounded-md border border-border text-muted-foreground transition-colors hover:text-foreground disabled:opacity-35"
            disabled={!previous}
            onClick={() => previous && onSelect(previous.id)}
            title="Previous event"
            type="button"
          >
            <ChevronLeft aria-hidden="true" className="h-3.5 w-3.5" />
          </button>
          <button
            aria-label="Next event"
            className="inline-flex h-6 w-6 items-center justify-center rounded-md border border-border text-muted-foreground transition-colors hover:text-foreground disabled:opacity-35"
            disabled={!next}
            onClick={() => next && onSelect(next.id)}
            title="Next event"
            type="button"
          >
            <ChevronRight aria-hidden="true" className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
      <fieldset
        className="grid min-w-0 gap-0.5 border-0 p-0"
        style={{
          gridTemplateColumns: `repeat(${Math.max(steps.length, 1)}, minmax(0, 1fr))`,
        }}
      >
        <legend className="sr-only">{steps.length} model-routed events</legend>
        {steps.map((step, index) => (
          <button
            aria-current={index === selectedIndex ? "step" : undefined}
            aria-label={`Event ${index + 1}: ${step.trace.name}, ${step.mode} mode`}
            className={cn(
              "h-4 min-w-0 rounded-[2px] transition-opacity",
              modeRailClassName(step.mode),
              index < selectedIndex ? "opacity-70" : "opacity-30",
              index === selectedIndex &&
                "opacity-100 ring-1 ring-foreground ring-offset-1 ring-offset-background",
            )}
            key={step.id}
            onClick={() => onSelect(step.id)}
            title={`${index + 1} · ${step.trace.name} · ${MODE_PRESENTATION[step.mode].label}`}
            type="button"
          />
        ))}
      </fieldset>
      <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-3xs text-muted-foreground">
        {(["radar", "explore", "steer"] as const).map((mode) => (
          <span className="inline-flex items-center gap-1" key={mode}>
            <span
              aria-hidden="true"
              className={cn(
                "h-1.5 w-1.5 rounded-[2px]",
                modeRailClassName(mode),
              )}
            />
            {MODE_PRESENTATION[mode].label}
          </span>
        ))}
      </div>
    </section>
  );
}

function CurrentEventHeader({
  index,
  step,
  total,
}: {
  index: number;
  step: ModelWorkStep;
  total: number;
}) {
  const presentation = MODE_PRESENTATION[step.mode];
  const Icon = presentation.icon;

  return (
    <div className="flex min-w-0 flex-col gap-2 border-b border-border/70 py-3 sm:flex-row sm:items-start sm:justify-between">
      <div className="min-w-0">
        <p className="text-3xs font-semibold uppercase text-muted-foreground/60">
          {phaseLabel(step)} · Event {index + 1} of {total}
        </p>
        <p
          className="mt-1 truncate text-sm font-semibold text-foreground"
          title={step.trace.name}
        >
          {step.trace.name}
        </p>
        <p className="mt-0.5 line-clamp-2 text-xs leading-4 text-muted-foreground">
          {step.detail ?? step.label}
        </p>
      </div>
      <span
        className={cn(
          "inline-flex h-6 shrink-0 items-center gap-1.5 rounded-md border px-2 text-2xs font-semibold uppercase",
          modeChipClassName(step.mode),
        )}
      >
        <Icon aria-hidden="true" className="h-3 w-3" />
        {presentation.label}
      </span>
    </div>
  );
}

function ModeRouter({ mode }: { mode: ModelWorkMode }) {
  return (
    <div className="grid min-w-0 grid-cols-[3rem_0.75rem_minmax(0,1fr)_0.75rem_4rem] items-center gap-1.5 border-y border-border/60 py-2.5">
      <div className="text-center text-3xs text-muted-foreground">
        <span className="block font-semibold text-foreground">Event</span>
        current
      </div>
      <ChevronRight
        aria-hidden="true"
        className="h-3 w-3 text-muted-foreground/45"
      />
      <div className="grid min-w-0 grid-cols-3 gap-1.5">
        {(["radar", "explore", "steer"] as const).map((candidate) => {
          const presentation = MODE_PRESENTATION[candidate];
          return (
            <div
              className={cn(
                "min-w-0 rounded-md border border-border bg-muted/10 px-1 py-2 text-center text-3xs text-muted-foreground",
                candidate === mode && modeNodeClassName(candidate),
              )}
              data-mode={candidate}
              key={candidate}
            >
              <span className="block truncate font-semibold text-foreground">
                {presentation.label}
              </span>
              {presentation.action}
            </div>
          );
        })}
      </div>
      <ChevronRight
        aria-hidden="true"
        className="h-3 w-3 text-muted-foreground/45"
      />
      <div className="text-center text-3xs text-muted-foreground">
        <span className="block font-semibold text-foreground">Harness</span>
        merge
      </div>
    </div>
  );
}

function ModeEnvelope({ step }: { step: ModelWorkStep }) {
  return (
    <dl className="grid min-w-0 grid-cols-1 border-b border-border/60 sm:grid-cols-2">
      <div className="min-w-0 py-2.5 sm:pr-2.5">
        <dt className="text-3xs font-semibold uppercase text-muted-foreground/60">
          Context received
        </dt>
        <dd className="mt-1 min-w-0 break-words font-mono text-2xs leading-4 text-muted-foreground">
          {step.trace.input ?? "shared run state"}
        </dd>
      </div>
      <div className="min-w-0 border-t border-border/60 py-2.5 sm:border-l sm:border-t-0 sm:pl-2.5">
        <dt className="text-3xs font-semibold uppercase text-muted-foreground/60">
          Result published
        </dt>
        <dd className="mt-1 min-w-0 break-words font-mono text-2xs leading-4 text-muted-foreground">
          {step.trace.output ??
            (step.status === "active" ? "Awaiting result" : step.label)}
        </dd>
      </div>
    </dl>
  );
}

function CurrentOperation({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  profiles,
  step,
}: ViewIdentityProps & { step: ModelWorkStep }) {
  const rendersExistingActivity =
    step.item.type === "tool" || step.item.type === "lifecycle";

  return (
    <div className="min-w-0">
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
          <p className="text-xs font-semibold text-foreground">{step.label}</p>
          {step.detail ? (
            <p className="mt-0.5 text-xs leading-4 text-muted-foreground">
              {step.detail}
            </p>
          ) : null}
        </div>
      )}
      <ActualTrace step={step} />
      {step.finding ? (
        <div className="mt-2 flex min-w-0 items-start gap-1.5 border-l-2 border-status-added/35 pl-2 text-xs leading-4 text-muted-foreground">
          <Sparkles
            aria-hidden="true"
            className="mt-0.5 h-3 w-3 shrink-0 text-status-added"
          />
          <p className="min-w-0">
            <span className="font-semibold text-foreground">
              {step.signalLabel}
            </span>
            {" · "}
            {step.finding}
          </p>
        </div>
      ) : null}
    </div>
  );
}

type HarnessAggregateValue = {
  actions: number;
  facts: number;
  hypotheses: number;
  latest: { id: string; label: string; mode: ModelWorkMode }[];
};

function HarnessAggregate({ aggregate }: { aggregate: HarnessAggregateValue }) {
  return (
    <aside
      className="min-w-0 border-t border-border/70 pt-3 md:border-l md:border-t-0 md:pl-4 md:pt-0"
      aria-label="Harness aggregated state"
    >
      <SectionHeader detail="Shared across modes" title="Harness aggregate" />
      <dl className="grid grid-cols-3 border-y border-border/60 py-2.5">
        <AggregateMetric label="Facts" value={aggregate.facts} />
        <AggregateMetric label="Hypotheses" value={aggregate.hypotheses} />
        <AggregateMetric label="Effects" value={aggregate.actions} />
      </dl>
      <div className="divide-y divide-border/60">
        {aggregate.latest.length > 0 ? (
          aggregate.latest.map((item) => (
            <div className="py-2" key={item.id}>
              <p
                className={cn(
                  "text-3xs font-semibold uppercase",
                  modeTextClassName(item.mode),
                )}
              >
                {MODE_PRESENTATION[item.mode].label} result
              </p>
              <p className="mt-0.5 line-clamp-2 text-xs leading-4 text-muted-foreground">
                {item.label}
              </p>
            </div>
          ))
        ) : (
          <p className="py-3 text-xs text-muted-foreground">
            Awaiting a published result.
          </p>
        )}
      </div>
    </aside>
  );
}

function AggregateMetric({ label, value }: { label: string; value: number }) {
  return (
    <div className="min-w-0">
      <dt className="truncate text-3xs text-muted-foreground">{label}</dt>
      <dd className="mt-0.5 text-sm font-semibold tabular-nums text-foreground">
        {value}
      </dd>
    </div>
  );
}

function CommunicationLog({ steps }: { steps: ModelWorkStep[] }) {
  return (
    <section className="pt-3" aria-label="Mode communication log">
      <SectionHeader
        detail="Latest handoffs and merged results"
        title="Communication through the harness"
      />
      <div className="divide-y divide-border/60 border-y border-border/60">
        {steps.map((step) => (
          <div
            className="grid min-w-0 grid-cols-[5.5rem_minmax(0,1fr)] gap-2 py-2 text-2xs sm:grid-cols-[6.5rem_7rem_minmax(0,1fr)]"
            key={step.id}
          >
            <span className={cn("font-semibold", modeTextClassName(step.mode))}>
              {step.mode} → harness
            </span>
            <code
              className="hidden truncate text-muted-foreground sm:block"
              title={step.trace.name}
            >
              {step.trace.name}
            </code>
            <span className="line-clamp-2 min-w-0 text-muted-foreground">
              {step.trace.output ?? step.finding ?? step.label}
            </span>
          </div>
        ))}
      </div>
    </section>
  );
}

function SectionHeader({ detail, title }: { detail: string; title: string }) {
  return (
    <div className="mb-2 flex min-w-0 items-center justify-between gap-3">
      <h4 className="text-xs font-semibold text-foreground">{title}</h4>
      <span className="truncate text-3xs text-muted-foreground">{detail}</span>
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

function buildHarnessAggregate(
  steps: readonly ModelWorkStep[],
): HarnessAggregateValue {
  const facts = steps.filter(
    (step) => step.mode === "radar" || Boolean(step.finding),
  ).length;
  const hypotheses = steps.filter(
    (step) => step.mode === "explore" && Boolean(step.finding),
  ).length;
  const actions = steps.filter(
    (step) => step.mode === "steer" && step.status === "complete",
  ).length;
  const latest = steps
    .filter(
      (step) =>
        step.status === "complete" &&
        Boolean(step.trace.output ?? step.finding ?? step.label),
    )
    .slice(-4)
    .reverse()
    .map((step) => ({
      id: step.id,
      label: step.trace.output ?? step.finding ?? step.label,
      mode: step.mode,
    }));

  return { actions, facts, hypotheses, latest };
}

function phaseLabel(step: ModelWorkStep) {
  if (step.mode === "radar") return "Observe";
  if (step.mode === "explore") return "Investigate";
  return step.phase === "deliver" ? "Deliver" : "Execute";
}

function modeRailClassName(mode: ModelWorkMode) {
  if (mode === "radar") return "bg-primary";
  if (mode === "explore") return "bg-warning";
  return "bg-status-added";
}

function modeTextClassName(mode: ModelWorkMode) {
  if (mode === "radar") return "text-primary";
  if (mode === "explore") return "text-warning";
  return "text-status-added";
}

function modeChipClassName(mode: ModelWorkMode) {
  if (mode === "radar") return "border-primary/40 bg-primary/10 text-primary";
  if (mode === "explore") return "border-warning/40 bg-warning-bg text-warning";
  return "border-status-added/40 bg-status-added/10 text-status-added";
}

function modeNodeClassName(mode: ModelWorkMode) {
  if (mode === "radar") return "border-primary/50 bg-primary/10 text-primary";
  if (mode === "explore") return "border-warning/50 bg-warning-bg text-warning";
  return "border-status-added/50 bg-status-added/10 text-status-added";
}
