import * as React from "react";
import { Check, CircleAlert, Loader2 } from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useControlledDisclosure } from "@/shared/hooks/useControlledDisclosure";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { AnimatedCount } from "@/shared/ui/AnimatedCount";
import {
  ActivityRow,
  ActivityRowContent,
  ActivityRowLabel,
  splitActivityRowCountedObject,
  type ActivityRowStats,
} from "./activityRenderClasses/ActivityRow";
import { TranscriptActivityItem } from "./activityRenderClasses/TranscriptActivityItem";
import { TranscriptTimestamp } from "./activityRenderClasses/TranscriptTimestamp";
import type { AgentTranscriptIdentityProps } from "./activityRenderClasses/types";
import { useAgentSessionTranscriptVariant } from "./agentSessionTranscriptContext";
import type { TranscriptToolRun } from "./agentSessionTranscriptGrouping";
import { buildCompactToolSummary } from "./agentSessionToolSummary";
import {
  summarizeToolRunHeadline,
  summarizeToolRunStatus,
  toolRunCompletedAtMs,
  toolRunElapsedMs,
  toolRunStartedAtMs,
  type ToolRunPhase,
} from "./agentSessionToolRunSummary";
import {
  formatDurationMs,
  formatTranscriptTimestampTitle,
} from "./agentSessionUtils";
import { hasFileEditLineDiff } from "./FileEditDiffView";
import { useTranscriptAnimationEnabled } from "./transcriptAnimationPreference";
import { useTranscriptTimestampsEnabled } from "./transcriptTimestampPreference";

type ToolRunStep = TranscriptToolRun["items"][number];

/**
 * Cadence of the live elapsed clock. Only mounted while a run is executing, so
 * a settled transcript never ticks.
 */
const LIVE_ELAPSED_TICK_MS = 1000;

/**
 * A run of consecutive tool steps, in whichever presentation the current
 * transcript variant calls for.
 *
 * This is the ONE place the variant is consulted: variant in, presentation out.
 * Grouping stays variant-agnostic — a run is a run everywhere — and keeping the
 * branch here rather than inside the card is what stops a second variant-aware
 * grouping pipeline from growing back.
 */
export function AgentSessionToolRunSegment({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  profiles,
  run,
}: AgentTranscriptIdentityProps & {
  profiles?: UserProfileLookup;
  run: TranscriptToolRun;
}) {
  const variant = useAgentSessionTranscriptVariant();
  const Presentation =
    variant === "compactPreview"
      ? AgentSessionToolRunCompactRow
      : AgentSessionToolRunCard;

  return (
    <Presentation
      agentAvatarUrl={agentAvatarUrl}
      agentName={agentName}
      agentPubkey={agentPubkey}
      profiles={profiles}
      run={run}
    />
  );
}

/**
 * One card for one run of consecutive tool steps.
 *
 * The card is the run's single row: it mutates in place as steps stream in
 * (VISION_ACTIVITY "mutate in place") rather than leaving a trail of status
 * rows. Its header is the run's verb/object/outcome sentence plus an aggregate
 * status glyph and timing; its body is one row per step, each reusing the
 * ordinary tool item rendering so shell blocks, diffs, sent-message previews,
 * and image previews all keep working.
 *
 * This is the full-fidelity presentation. The compact activity preview renders
 * a run as a plain summary row instead; that choice is made once, at the
 * transcript list boundary, so nothing here is variant-aware.
 */
export function AgentSessionToolRunCard({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  profiles,
  run,
}: AgentTranscriptIdentityProps & {
  profiles?: UserProfileLookup;
  run: TranscriptToolRun;
}) {
  const timestampsEnabled = useTranscriptTimestampsEnabled();
  const aggregate = summarizeToolRunStatus(run.items);
  const headline = summarizeToolRunHeadline(run.items, aggregate);
  const stats = useToolRunEditStats(run.items);
  const { onOpenChange, open } = useToolRunDisclosure(aggregate.phase);

  return (
    <div data-run-phase={aggregate.phase} data-tool-run-id={run.id}>
      <ActivityRow
        className="flex flex-col gap-0.5"
        onOpenChange={onOpenChange}
        open={open}
        openToneScope="summary"
        testId="transcript-tool-run-card"
        title={formatTranscriptTimestampTitle(run.timestamp)}
      >
        <ToolRunStatusGlyph
          errorCount={aggregate.errorCount}
          phase={aggregate.phase}
        />
        <ToolRunHeadlineLabel
          detail={headline.detail}
          object={headline.object}
          stats={stats}
          verb={headline.verb}
        />
        <ToolRunTiming
          isRunning={aggregate.phase === "running"}
          items={run.items}
        />
        <ActivityRowContent className="flex flex-col gap-0.5">
          {run.items.map((item) => (
            <ToolRunStepRow
              agentAvatarUrl={agentAvatarUrl}
              agentName={agentName}
              agentPubkey={agentPubkey}
              item={item}
              key={item.id}
              profiles={profiles}
            />
          ))}
        </ActivityRowContent>
      </ActivityRow>
      {timestampsEnabled ? (
        <div
          className="mt-0.5 flex justify-start"
          data-testid="transcript-row-timestamp"
        >
          <TranscriptTimestamp timestamp={run.timestamp} />
        </div>
      ) : null}
    </div>
  );
}

/**
 * A run in the compact activity preview: one plain, collapsed, self-managed
 * row.
 *
 * The preview is a non-interactive thumbnail of what an agent is doing, not a
 * place to supervise it — so a run reads there exactly as it did before chain
 * cards existed: the generic `Ran N tool calls` sentence the legacy mixed-burst
 * fallthrough used, with no aggregate status glyph, no derived verb/object
 * headline ("Read 2 files"), no live clock, no timing, and no
 * auto-open/auto-collapse policy. Disclosure is the `<details>` element's own
 * business, so expanding stays possible and stays entirely the reader's choice.
 */
export function AgentSessionToolRunCompactRow({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  profiles,
  run,
}: AgentTranscriptIdentityProps & {
  profiles?: UserProfileLookup;
  run: TranscriptToolRun;
}) {
  return (
    <ActivityRow
      className="flex flex-col gap-0.5"
      openToneScope="summary"
      testId="transcript-tool-run-compact-row"
      title={formatTranscriptTimestampTitle(run.timestamp)}
    >
      <ToolRunHeadlineLabel
        detail={null}
        object={`${run.items.length} tool calls`}
        stats={null}
        verb="Ran"
      />
      <ActivityRowContent className="flex flex-col gap-0.5">
        {run.items.map((item) => (
          <ToolRunStepRow
            agentAvatarUrl={agentAvatarUrl}
            agentName={agentName}
            agentPubkey={agentPubkey}
            item={item}
            key={item.id}
            profiles={profiles}
          />
        ))}
      </ActivityRowContent>
    </ActivityRow>
  );
}

/**
 * Disclosure policy for a run card.
 *
 * A live run is open so the reader watches work happen; when it settles the
 * card collapses itself and hands the space back to the conversation — unless
 * the run failed, in which case it stays open (a buried error is a broken
 * feed). `useControlledDisclosure` layers the reader's own toggle over that
 * policy and guards the `<details>` echo trap.
 */
function useToolRunDisclosure(phase: ToolRunPhase) {
  return useControlledDisclosure(phase !== "done");
}

/**
 * Aggregate +/- across the run's real line diffs. Runs that edited nothing
 * report no stats rather than a misleading "+0 -0".
 */
function useToolRunEditStats(items: ToolRunStep[]): ActivityRowStats | null {
  return React.useMemo(() => {
    let additions = 0;
    let deletions = 0;
    let sawDiff = false;

    for (const item of items) {
      if (item.isError) continue;
      const diff = buildCompactToolSummary(item).fileEditDiff;
      if (!diff || !hasFileEditLineDiff(diff)) continue;
      sawDiff = true;
      additions += diff.additions;
      deletions += diff.deletions;
    }

    return sawDiff ? { additions, deletions } : null;
  }, [items]);
}

/**
 * Aggregate outcome glyph: spinner while any step executes, check when the run
 * finished clean, error mark when any step failed.
 */
function ToolRunStatusGlyph({
  errorCount,
  phase,
}: {
  errorCount: number;
  phase: ToolRunPhase;
}) {
  if (phase === "running") {
    return (
      <>
        <Loader2
          aria-hidden="true"
          className="h-3.5 w-3.5 shrink-0 animate-spin text-muted-foreground/70"
        />
        <span className="sr-only">Running</span>
      </>
    );
  }

  if (phase === "error") {
    return (
      <>
        <CircleAlert
          aria-hidden="true"
          className="h-3.5 w-3.5 shrink-0 text-destructive"
        />
        <span className="sr-only">
          {errorCount === 1 ? "1 step failed" : `${errorCount} steps failed`}
        </span>
      </>
    );
  }

  return (
    <>
      <Check
        aria-hidden="true"
        className="h-3.5 w-3.5 shrink-0 text-muted-foreground/70"
      />
      <span className="sr-only">Done</span>
    </>
  );
}

/**
 * The run's headline sentence. A leading count in the object phrase rolls
 * through AnimatedCount so a run growing from "Read 3 files" to "Read 4 files"
 * reads as an increment rather than a silent swap.
 */
function ToolRunHeadlineLabel({
  detail,
  object,
  stats,
  verb,
}: {
  detail: string | null;
  object: string | null;
  stats: ActivityRowStats | null;
  verb: string;
}) {
  const animationsEnabled = useTranscriptAnimationEnabled();
  const counted =
    animationsEnabled && object ? splitActivityRowCountedObject(object) : null;

  const objectNode = counted ? (
    <>
      <AnimatedCount value={counted.count} />
      {counted.rest}
    </>
  ) : (
    object
  );

  return (
    <>
      <ActivityRowLabel
        object={objectNode}
        openToneScope="summary"
        stats={stats}
        verb={verb}
      />
      {detail ? (
        <span className="shrink-0 text-xs text-muted-foreground/60">
          {detail}
        </span>
      ) : null}
    </>
  );
}

/**
 * Run timing: a live counter while the run executes, the finished span once it
 * settles. Split so only the live branch mounts the ticking clock.
 */
function ToolRunTiming({
  isRunning,
  items,
}: {
  isRunning: boolean;
  items: ToolRunStep[];
}) {
  if (isRunning) {
    return <ToolRunLiveElapsed items={items} />;
  }

  const startedAt = toolRunStartedAtMs(items);
  const completedAt = toolRunCompletedAtMs(items);
  // Nothing is executing but a step never reported a completion instant: the
  // span is unknowable, so report nothing rather than a number frozen against
  // whenever this happened to render.
  if (startedAt === null || completedAt === null) return null;

  return <ToolRunTimingText ms={completedAt - startedAt} />;
}

function ToolRunLiveElapsed({ items }: { items: ToolRunStep[] }) {
  const now = useNow(LIVE_ELAPSED_TICK_MS);
  return <ToolRunTimingText ms={toolRunElapsedMs(items, now)} />;
}

function ToolRunTimingText({ ms }: { ms: number | null }) {
  const formatted = ms === null ? null : formatDurationMs(ms);
  if (!formatted) return null;

  return (
    <span className="shrink-0 text-xs text-muted-foreground/60 tabular-nums">
      {formatted}
    </span>
  );
}

/**
 * One step inside an expanded run. Reuses the ordinary tool item rendering so
 * every render class keeps its own presentation; a failed step additionally
 * carries a left rule so the eye lands on it without hunting.
 *
 * Memoized on the step's identity, because a run is re-rendered on **every**
 * append while it streams and on every live-clock tick. Without this, appending
 * one step to a run of twenty re-runs compact-summary building, diff parsing,
 * and markdown/image rendering for all twenty unchanged steps — work the
 * transcript's own `TranscriptItemView` avoids the same way. Transcript items
 * are replaced rather than mutated, so reference equality on `item` is a sound
 * test for "this step did not change".
 */
const ToolRunStepRow = React.memo(function ToolRunStepRow({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  item,
  profiles,
}: AgentTranscriptIdentityProps & {
  item: ToolRunStep;
  profiles?: UserProfileLookup;
}) {
  const failed = item.isError || item.status === "failed";

  return (
    <div
      className={cn(
        "min-w-0",
        failed &&
          "rounded-sm border-l-2 border-destructive/60 bg-destructive/5 pl-1.5",
      )}
      data-step-failed={failed ? "true" : undefined}
      data-testid="transcript-tool-run-step"
    >
      <TranscriptActivityItem
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        item={item}
        profiles={profiles}
      />
    </div>
  );
});
