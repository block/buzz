import * as React from "react";
import { AnimatePresence, motion } from "motion/react";
import { ChevronRight, Circle, MessageCircle, Wrench } from "lucide-react";

import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";
import { CodeBlockVariantContext } from "@/shared/ui/markdown/CodeBlock";
import {
  AgentSessionWorkBlockRailProvider,
  useAgentSessionTranscriptTurnMeta,
} from "./agentSessionTranscriptContext";
import { useTranscriptAnimationEnabled } from "./transcriptAnimationPreference";
import {
  readWorkBlockChoice,
  useWorkBlockDisclosureState,
  useWorkBlockDisclosureStore,
} from "./agentSessionWorkBlockDisclosure";
import { formatTranscriptTimestampTitle } from "./agentSessionUtils";
import { TranscriptActivityItem } from "./activityRenderClasses/TranscriptActivityItem";
import type { AgentTranscriptIdentityProps } from "./activityRenderClasses/types";
import {
  formatPreviousStepsLabel,
  formatWorkBlockSummaryLabel,
  projectWorkBlockEntries,
  summarizeWorkBlock,
  windowWorkBlockEntries,
  type TranscriptWorkBlock,
  type WorkBlockEntry,
  type WorkBlockStatus,
} from "./agentSessionWorkBlockGrouping";

const FADE_TRANSITION = {
  duration: 0.18,
  ease: [0.215, 0.61, 0.355, 1],
} as const;
const COLLAPSE_TRANSITION = {
  duration: 0.22,
  ease: [0.215, 0.61, 0.355, 1],
} as const;

const REDUCED_MOTION_QUERY = "(prefers-reduced-motion: reduce)";

/**
 * The OS reduced-motion preference, read through `matchMedia` the way the rest
 * of the app reads it (see `TerminalSubstrate`) rather than through motion's
 * `useReducedMotion`.
 *
 * That helper resolves the query once per process and caches the answer, so it
 * reports whatever the first render happened to see for the lifetime of the
 * process. Here that would mean the reduced-motion path could never be
 * exercised under test — the assertion would pass or fail on module load order
 * rather than on the preference — and a preference change mid-session would be
 * ignored. Subscribing to the query keeps both honest.
 */
function usePrefersReducedMotion() {
  const [reduced, setReduced] = React.useState(
    () => window.matchMedia(REDUCED_MOTION_QUERY).matches,
  );

  React.useEffect(() => {
    const preference = window.matchMedia(REDUCED_MOTION_QUERY);
    const update = () => setReduced(preference.matches);
    update();
    preference.addEventListener("change", update);
    return () => preference.removeEventListener("change", update);
  }, []);

  return reduced;
}

/**
 * Whether the reader wants motion at all: the transcript's own animation
 * preference AND the OS reduced-motion setting. Both are respected, so a
 * height animation is skipped rather than shortened when either says no.
 */
function useWorkBlockMotionEnabled() {
  const animationPreferenceEnabled = useTranscriptAnimationEnabled();
  const prefersReducedMotion = usePrefersReducedMotion();
  return animationPreferenceEnabled && !prefersReducedMotion;
}

/**
 * Animate a disclosure's height between 0 and its natural height.
 *
 * `<details>` cannot do this — its content is either laid out or not, with no
 * intermediate height to tween — so the collapse would simply snap. Height is
 * animated to and from `auto`, which motion resolves by measuring, so the block
 * never needs a hard-coded height that could disagree with its content.
 */
function DisclosurePanel({
  children,
  motionEnabled,
  open,
}: {
  children: React.ReactNode;
  motionEnabled: boolean;
  open: boolean;
}) {
  if (!motionEnabled) {
    return open ? <div>{children}</div> : null;
  }

  return (
    <AnimatePresence initial={false}>
      {open ? (
        <motion.div
          animate={{ height: "auto", opacity: 1 }}
          className="overflow-hidden"
          exit={{ height: 0, opacity: 0 }}
          initial={{ height: 0, opacity: 0 }}
          transition={COLLAPSE_TRANSITION}
        >
          {children}
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}

/**
 * Whether a finished block should play its collapse on first paint.
 *
 * A turn that finishes while the reader is watching should be *seen* to fold —
 * otherwise the rail they were reading is simply replaced by a one-line summary
 * between frames, and it reads as content disappearing rather than as work
 * settling. So a block that was live when it mounted renders open once, then
 * closes after a paint, giving the height animation a real open→closed
 * transition to play.
 *
 * A block that was already finished when it mounted (history load, scrollback)
 * never had a rail on screen, so there is nothing to fold: it renders closed
 * immediately. Distinguishing the two is what keeps the animation meaningful
 * instead of a spurious flourish on every mount.
 */
function useSettleOnFinish(isActive: boolean, motionEnabled: boolean) {
  const [settling, setSettling] = React.useState(false);
  const wasActiveRef = React.useRef(isActive);

  React.useEffect(() => {
    if (isActive) {
      wasActiveRef.current = true;
      setSettling(false);
      return;
    }
    if (!wasActiveRef.current) return;
    wasActiveRef.current = false;

    if (!motionEnabled) {
      setSettling(false);
      return;
    }

    // Two frames: the first commits the still-open render, the second flips it
    // closed, so the animation has a start state that was actually painted.
    setSettling(true);
    let second: number | null = null;
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() => setSettling(false));
    });
    return () => {
      cancelAnimationFrame(first);
      if (second !== null) cancelAnimationFrame(second);
    };
  }, [isActive, motionEnabled]);

  return settling;
}

/**
 * Whether the block renders open, and how the reader takes that decision over.
 *
 * The policy is "open while work is in flight, folded once it finishes". The
 * reader's own click wins from then on.
 *
 * The reader's choice is held ABOVE this component, keyed by the step ids it was
 * taken on, because a work block does not outlive its own membership: blocks are
 * regrouped from scratch each render and merge when a second assistant message
 * demotes the first from final answer, so a `useState` here is destroyed by the
 * agent posting again. See `agentSessionWorkBlockDisclosure`.
 *
 * The trigger is deliberately button-native. An earlier version used a shared
 * `useControlledDisclosure` hook whose whole reason to exist was the
 * `<details>` echo trap: `<details>` fires `toggle` for programmatic `open`
 * changes as well as for clicks, indistinguishably, so a policy-driven open
 * echoes back looking like reader intent and pins the row to its first policy
 * state forever. This block's trigger is a `<button>`, so the only thing that
 * can call `onToggle` is a real click — there is no echo to discriminate, and a
 * guard against it would be dead code that reads as though it were load-bearing.
 */
function useWorkBlockDisclosure(
  policyOpen: boolean,
  itemIds: readonly string[],
) {
  // Local state is the fallback for a block mounted outside a transcript, where
  // nothing regroups it and per-component state is correct. Both hooks always
  // run; only one of them is read.
  const localStore = useWorkBlockDisclosureState();
  const { choices, choose } = useWorkBlockDisclosureStore() ?? localStore;
  const readerChoice = readWorkBlockChoice(choices, itemIds);
  const open = readerChoice ?? policyOpen;

  return {
    chosenByReader: readerChoice !== null,
    open,
    toggle: React.useCallback(
      () => choose(itemIds, !open),
      [choose, itemIds, open],
    ),
  };
}

export function AgentSessionWorkBlockSegment({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  block,
  profiles,
}: AgentTranscriptIdentityProps & {
  block: TranscriptWorkBlock;
  profiles?: UserProfileLookup;
}) {
  const turnMeta = useAgentSessionTranscriptTurnMeta();
  // One projection per render, shared by the summary counts and the rail, so
  // "is this step failed?" has exactly one answer in this component.
  const entries = React.useMemo(
    () =>
      projectWorkBlockEntries(block.items, {
        liveTurnId: turnMeta.liveTurnId,
      }),
    [block.items, turnMeta.liveTurnId],
  );
  const status = React.useMemo(
    () =>
      summarizeWorkBlock(entries, {
        streamingItemId: turnMeta.streamingItemId,
      }),
    [entries, turnMeta.streamingItemId],
  );
  const motionEnabled = useWorkBlockMotionEnabled();
  const settling = useSettleOnFinish(status.isActive, motionEnabled);

  // The steps the reader's fold choice is recorded against. Memoized on the
  // items so `toggle`'s identity is stable between renders that did not change
  // membership.
  const itemIds = React.useMemo(
    () => block.items.map((item) => item.id),
    [block.items],
  );
  const disclosure = useWorkBlockDisclosure(
    status.isActive || settling,
    itemIds,
  );
  const isLive = status.isActive;
  // A reader who expanded the block asked to see all of it, so windowing stops
  // applying. The test is the reader's *choice*, not `open`: policy already
  // holds every live block open, so keying this off `open` would switch
  // windowing off in exactly the case it exists for.
  const readerExpanded = disclosure.chosenByReader && disclosure.open;
  const { hiddenEntries, visibleEntries } = React.useMemo(
    () =>
      windowWorkBlockEntries(entries, {
        isActive: isLive && !readerExpanded,
      }),
    [entries, isLive, readerExpanded],
  );

  return (
    // Declared once at the block root rather than per row: every tool item
    // rendered anywhere inside a block — windowed rail, previous-steps
    // disclosure — is a rail step, and the value is constant so it adds no
    // re-render of its own.
    <AgentSessionWorkBlockRailProvider value={true}>
      <div
        className="w-full min-w-0"
        data-role="agent-work-block"
        data-testid="transcript-work-block"
        title={formatTranscriptTimestampTitle(block.timestamp)}
      >
        {/* While live the rail IS the status: a header would only restate what
          the arriving steps already show. */}
        {isLive ? null : (
          <WorkBlockSummaryTrigger
            onToggle={disclosure.toggle}
            open={disclosure.open}
            status={status}
          />
        )}
        <DisclosurePanel
          motionEnabled={motionEnabled}
          open={isLive || disclosure.open}
        >
          <div className="min-h-0">
            {hiddenEntries.length > 0 ? (
              <PreviousStepsDisclosure
                agentAvatarUrl={agentAvatarUrl}
                agentName={agentName}
                agentPubkey={agentPubkey}
                entries={hiddenEntries}
                motionEnabled={motionEnabled}
                profiles={profiles}
              />
            ) : null}
            <WorkBlockRail
              agentAvatarUrl={agentAvatarUrl}
              agentName={agentName}
              agentPubkey={agentPubkey}
              animateEnter={motionEnabled && isLive}
              entries={visibleEntries}
              profiles={profiles}
            />
          </div>
        </DisclosurePanel>
      </div>
    </AgentSessionWorkBlockRailProvider>
  );
}

function WorkBlockSummaryTrigger({
  onToggle,
  open,
  status,
}: {
  onToggle: () => void;
  open: boolean;
  status: WorkBlockStatus;
}) {
  return (
    <button
      aria-expanded={open}
      className="group/row flex min-h-6 w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-md py-1 text-left text-muted-foreground transition-colors hover:text-foreground"
      data-testid="transcript-work-block-summary"
      onClick={onToggle}
      type="button"
    >
      <span className="min-w-0 truncate text-sm tabular-nums">
        {formatWorkBlockSummaryLabel(status)}
      </span>
      <ChevronRight
        aria-hidden="true"
        className={cn(
          "h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform group-hover/row:text-foreground",
          open && "rotate-90",
        )}
      />
    </button>
  );
}

function PreviousStepsDisclosure({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  entries,
  motionEnabled,
  profiles,
}: AgentTranscriptIdentityProps & {
  entries: WorkBlockEntry[];
  motionEnabled: boolean;
  profiles?: UserProfileLookup;
}) {
  const [open, setOpen] = React.useState(false);

  return (
    <motion.div
      animate={{ opacity: 1 }}
      initial={motionEnabled ? { opacity: 0 } : false}
      transition={motionEnabled ? FADE_TRANSITION : { duration: 0 }}
    >
      <button
        aria-expanded={open}
        className="group/row mb-1 flex min-h-6 w-full min-w-0 cursor-pointer items-center gap-1.5 rounded-md py-1 text-left text-muted-foreground transition-colors hover:text-foreground"
        data-testid="transcript-work-block-previous-steps"
        onClick={() => setOpen((previous) => !previous)}
        type="button"
      >
        <span className="min-w-0 truncate text-sm tabular-nums">
          {formatPreviousStepsLabel(entries.length)}
        </span>
        <ChevronRight
          aria-hidden="true"
          className={cn(
            "h-3.5 w-3.5 shrink-0 text-muted-foreground/60 transition-transform group-hover/row:text-foreground",
            open && "rotate-90",
          )}
        />
      </button>
      <DisclosurePanel motionEnabled={motionEnabled} open={open}>
        <WorkBlockRail
          agentAvatarUrl={agentAvatarUrl}
          agentName={agentName}
          agentPubkey={agentPubkey}
          animateEnter={false}
          entries={entries}
          profiles={profiles}
        />
      </DisclosurePanel>
    </motion.div>
  );
}

function WorkBlockRail({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  animateEnter,
  entries,
  profiles,
}: AgentTranscriptIdentityProps & {
  animateEnter: boolean;
  entries: WorkBlockEntry[];
  profiles?: UserProfileLookup;
}) {
  return (
    <div data-role="agent-work-block-rail">
      <AnimatePresence initial={false} mode="popLayout">
        {entries.map((entry, index) => (
          <motion.div
            animate={{ opacity: 1, y: 0 }}
            exit={animateEnter ? { opacity: 0, y: -4 } : undefined}
            initial={animateEnter ? { opacity: 0, y: 4 } : false}
            key={entry.item.id}
            transition={animateEnter ? FADE_TRANSITION : { duration: 0 }}
          >
            <WorkBlockStepRow
              agentAvatarUrl={agentAvatarUrl}
              agentName={agentName}
              agentPubkey={agentPubkey}
              entry={entry}
              isLast={index === entries.length - 1}
              profiles={profiles}
            />
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

function WorkBlockStepRow({
  agentAvatarUrl,
  agentName,
  agentPubkey,
  entry,
  isLast,
  profiles,
}: AgentTranscriptIdentityProps & {
  entry: WorkBlockEntry;
  isLast: boolean;
  profiles?: UserProfileLookup;
}) {
  return (
    <div
      className="flex gap-2.5"
      data-testid="transcript-work-block-step"
      data-work-block-entry={entry.kind}
    >
      <WorkBlockRailGlyph entry={entry} isLast={isLast} />
      <WorkBlockStepBody
        agentAvatarUrl={agentAvatarUrl}
        agentName={agentName}
        agentPubkey={agentPubkey}
        profiles={profiles}
        {...entry}
      />
    </div>
  );
}

/**
 * A step's content: one exhaustive switch over the entry kind, so a new kind
 * cannot silently inherit another kind's presentation.
 *
 * The entry arrives SPREAD into props rather than as an `entry` object, because
 * this component is memoized and the projection is rebuilt whenever the block's
 * item array changes — i.e. on every append — so entry objects are fresh each
 * time and a memo keyed on one would never hit. Spread, the compared props are
 * `item` (reference-stable: the transcript store replaces items rather than
 * mutating them) plus two strings, so shallow comparison is a sound "did not
 * change" test where comparison on the entry wrapper is not.
 *
 * Spreading keeps the discriminated union intact — `kind` and `item` stay paired
 * in the props type — so the switch below narrows `item` to the kind's own item
 * type. That is what removes the previous `item.type === "thought" ? ... : ""`
 * re-checks: a mismatch is no longer representable, so there is no mismatch
 * branch to render an empty body for.
 *
 * The memo boundary is the body rather than the whole row because the glyph
 * depends on the row's *position* (`isLast` decides whether the spine
 * continues), and that changes for the previous last row on every append.
 * Passing position into the memoized part would invalidate that row's body on
 * each append for a one-pixel spine segment; splitting keeps the cheap,
 * position-dependent half outside and the expensive, item-dependent half in.
 */
const WorkBlockStepBody = React.memo(function WorkBlockStepBody(
  props: WorkBlockEntryBodyProps,
) {
  return (
    <div className="min-w-0 flex-1 pb-2">
      <WorkBlockEntryBody {...props} />
    </div>
  );
});

type WorkBlockEntryBodyProps = AgentTranscriptIdentityProps & {
  profiles?: UserProfileLookup;
} & WorkBlockEntry;

/**
 * One exhaustive switch over the entry kind, so a new kind cannot silently
 * inherit another kind's presentation.
 *
 * Because the props carry the whole discriminated entry, narrowing on `kind`
 * also narrows `item` to that kind's item type. The previous version took
 * `kind` and `item` as independent fields and so had to re-check
 * `item.type === "thought" ? item.text : ""` — a mismatch branch that rendered
 * an empty body and could only ever be reached by a projection bug.
 */
function WorkBlockEntryBody(props: WorkBlockEntryBodyProps) {
  switch (props.kind) {
    case "thought":
      return (
        <WorkBlockProseBody
          testId="transcript-work-block-thought"
          text={props.item.text}
        />
      );
    case "note":
      return (
        <WorkBlockProseBody
          testId="transcript-work-block-note"
          text={props.item.text}
        />
      );
    case "tool":
      return (
        <TranscriptActivityItem
          agentAvatarUrl={props.agentAvatarUrl}
          agentName={props.agentName}
          agentPubkey={props.agentPubkey}
          item={props.item}
          profiles={props.profiles}
        />
      );
  }
}

/**
 * Prose on the rail: reasoning, or an interim note the agent addressed to the
 * reader. No disclosure of its own — the whole block is already one, and
 * nesting a second would mean two clicks to read something the reader has just
 * chosen to reveal.
 *
 * A note deliberately does NOT go through the message presenter. That presenter
 * renders assistant answers as standalone prose. Sending a note through that
 * presenter would still make a muted rail step read as a second reply rather
 * than as progress. berd draws the same distinction — its `progress` entry is a
 * plain rail row, not a message bubble. The focus code-block recipe is kept by
 * providing the same `CodeBlockVariantContext` value the presenter would.
 *
 * berd brightens rail prose with `usePrimaryText={open}`. Here it is
 * unconditional, because the rail only ever exists inside the open disclosure
 * panel — a closed block unmounts its rows entirely rather than rendering them
 * dimmed. Threading an `open` flag down to this component would be a prop whose
 * false branch is unreachable, which is the same shape of dead-code-that-looks-
 * load-bearing as the disclosure echo guard this block also dropped. If the
 * block ever renders a peek of its rows while closed, the flag comes back with
 * a test that can actually reach both branches.
 */
function WorkBlockProseBody({
  testId,
  text,
}: {
  testId: string;
  text: string;
}) {
  return (
    <div
      className="text-foreground text-sm leading-relaxed"
      data-testid={testId}
    >
      <CodeBlockVariantContext.Provider value="focusProse">
        <Markdown className="leading-relaxed" content={text.trim() || " "} />
      </CodeBlockVariantContext.Provider>
    </div>
  );
}

/**
 * The spine and this row's bullet.
 *
 * The bullet masks the spine passing behind it, which is what makes the rail
 * read as a series of stops rather than a line with icons floating over it.
 * The mask has to match the surface the transcript is actually drawn on — the
 * cover drawer's `bg-background` — because a mask in any other colour shows up
 * as a visible disc of the wrong shade around every bullet.
 *
 * (berd's equivalent uses `bg-card` and warns against `bg-background`; that is
 * the same rule, not a different one. In berd the transcript sits on a card, so
 * `bg-card` is its surface. Buzz's drawer surface is `bg-background`, and the
 * two tokens are NOT interchangeable here: they share a value in the base
 * themes, but in Buzz Dark the drawer sits inside `[data-buzz-content-surface]`,
 * which locally overrides `--background` to `--buzz-content-dark` while
 * `--card` keeps the theme value. Measured in a seeded browser:
 *
 *   | theme        | bullet          | drawer surface  | spine           |
 *   | ------------ | --------------- | --------------- | --------------- |
 *   | github-light | rgb(255,255,255)| rgb(255,255,255)| rgb(229,229,230)|
 *   | buzz-dark    | rgb(26,26,26)   | rgb(26,26,26)   | rgb(64,69,74)   |
 *
 * `bg-card` paints the bullet `rgb(36,41,46)` over that `rgb(26,26,26)` drawer
 * — a visible disc of the wrong shade, which is exactly the BOT-1599 failure.
 * Light mode alone matches under either class, so light-mode evidence is not
 * sufficient here. Following berd's class literally would be following the
 * letter of its note against its point.)
 */
function WorkBlockRailGlyph({
  entry,
  isLast,
}: {
  entry: WorkBlockEntry;
  isLast: boolean;
}) {
  return (
    <div
      aria-hidden="true"
      className="relative flex w-5 shrink-0 justify-center self-stretch"
    >
      {isLast ? null : (
        <div className="absolute top-5 bottom-0 left-1/2 w-px -translate-x-1/2 bg-border" />
      )}
      <div
        className={cn(
          "relative z-10 mt-0.5 flex size-5 items-center justify-center rounded-full bg-background text-muted-foreground ring-2 ring-background",
          // `motion-safe:`, not bare `animate-pulse`: the guard has to be in the
          // class because this animation is CSS, not motion. The block's height
          // animation is skipped through `useWorkBlockMotionEnabled`, but that
          // hook cannot reach a keyframe animation applied by a utility — and no
          // reduced-motion rule in the app's CSS matches `.animate-pulse` (every
          // one of them is scoped to a `buzz-*`/`motion-*` class), so an
          // unguarded pulse would keep pulsing forever for a reader who asked
          // for no motion. Tailwind compiles `motion-safe:` to
          // `@media (prefers-reduced-motion: no-preference)`, which is the only
          // thing that actually stops it. Matches every other pulse in this
          // feature (`AgentStatusBadge`, `ManagedAgentRow`).
          entry.state === "running" && "motion-safe:animate-pulse",
        )}
        data-step-state={entry.state}
      >
        <WorkBlockRailGlyphIcon entry={entry} />
      </div>
    </div>
  );
}

/** The glyph itself: shape carries kind, and for a tool step, outcome. */
function WorkBlockRailGlyphIcon({ entry }: { entry: WorkBlockEntry }) {
  switch (entry.kind) {
    // Prose — reasoning or an interim note — is the agent talking, so both get
    // the speech bubble. berd gives its `progress` entry the same glyph as a
    // thought for the same reason.
    case "thought":
    case "note":
      return <MessageCircle className="size-3.5" />;
    case "tool":
      return entry.state === "failed" ? (
        // Failure is a glyph shape, not a colour: the rail stays uniformly
        // muted so one failed step does not read as an alarm across the whole
        // run. The tinted output block carries the red when the step is
        // expanded.
        <Circle className="size-2.5 fill-current" />
      ) : (
        <Wrench className="size-3.5" />
      );
  }
}
