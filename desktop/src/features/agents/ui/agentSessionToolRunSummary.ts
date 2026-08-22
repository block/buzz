import type {
  AgentActivityDescriptor,
  AgentActivityRenderClass,
  AgentActivityTone,
  TranscriptItem,
} from "./agentSessionTypes";
import { classifyTool, classifyToolItem } from "./agentSessionToolClassifier";

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

/**
 * Which render classes may join a tool-chain run.
 *
 * Exhaustive by construction: adding a render class to
 * `AgentActivityRenderClass` forces a decision here rather than silently
 * opting the new class into (or out of) chaining.
 *
 * The exclusions are deliberate, not incidental:
 *  - `raw-rail` / `suppressed` — the ambient safety net. Collapsing ground
 *    truth or deliberately-unrendered noise into a chain would hide the two
 *    classes that exist precisely to be a floor.
 *  - `status` — turn/tool heartbeat (e.g. "Context compacted"). Spine content
 *    that changes how every later step should be read; it must not disappear
 *    into a collapsed card.
 *  - `permission` / `thought` — intervention points and reasoning. A control
 *    gate the reader may need to act on never collapses, and thoughts are read
 *    as prose rather than as steps.
 *
 * Everything else chains, including `message` (so sent-message previews keep
 * rendering inside a chain body) and `error` (so a failed step is a *member*
 * of the run that failed rather than a row that shatters it — the card then
 * stays open and highlights it).
 */
const CHAIN_ELIGIBLE_RENDER_CLASSES = {
  "file-read": true,
  "file-edit": true,
  "relay-op": true,
  "skill-read": true,
  message: true,
  generic: true,
  image: true,
  shell: true,
  plan: true,
  error: true,
  permission: false,
  "raw-rail": false,
  suppressed: false,
  thought: false,
  status: false,
} satisfies Record<AgentActivityRenderClass, boolean>;

/** Runs shorter than this render as ordinary standalone rows. */
export const TOOL_RUN_MINIMUM_STEPS = 2;

/**
 * Per-render-class phrasing floor: the past-tense verb to use when a
 * descriptor carries no `action`, and the singular noun for the run's object
 * phrase ("3 files", "2 Buzz relay ops").
 *
 * The classifier is the source of truth for *vocabulary* — every descriptor
 * already carries an `action.verb` derived from the tool and its tone
 * (`agentSessionToolClassifier`), and that verb is what a run headlines with.
 * This table only supplies what a per-step descriptor cannot: the collective
 * noun for a run of N steps, plus a verb floor for items whose descriptor
 * predates `action`. Exhaustive by construction, so a new render class forces a
 * decision here instead of silently headlining as generic tool work.
 */
const RENDER_CLASS_PHRASE = {
  "file-read": { countable: true, noun: "file", verb: "Read" },
  "file-edit": { countable: true, noun: "file", verb: "Edited" },
  "skill-read": { countable: true, noun: "skill", verb: "Read" },
  "relay-op": { countable: true, noun: "Buzz relay op", verb: "Ran" },
  message: { countable: true, noun: "message", verb: "Sent" },
  image: { countable: true, noun: "image", verb: "Viewed" },
  shell: { countable: true, noun: "command", verb: "Ran" },
  plan: { countable: true, noun: "todo", verb: "Updated" },
  // Not countable: a homogeneous run of unnamed work says what the classifier
  // called it ("Queried registry ×2") rather than counting anonymous "tool
  // calls"; the noun is only reached when such work dominates a mixed run.
  generic: { countable: false, noun: "tool call", verb: "Ran" },
  // Reached only by a failed step whose original classification could not be
  // recovered, so it reports honestly rather than guessing.
  error: { countable: false, noun: "tool call", verb: "Ran" },
  // Never chain (see CHAIN_ELIGIBLE_RENDER_CLASSES), so these are unreachable
  // here; they exist to keep the table exhaustive.
  permission: { countable: false, noun: "step", verb: "Ran" },
  "raw-rail": { countable: false, noun: "step", verb: "Ran" },
  suppressed: { countable: false, noun: "step", verb: "Ran" },
  thought: { countable: false, noun: "step", verb: "Ran" },
  status: { countable: false, noun: "step", verb: "Ran" },
} satisfies Record<
  AgentActivityRenderClass,
  { countable: boolean; noun: string; verb: RunVerb }
>;

/**
 * Every past-tense verb a run can headline with *of this module's own choosing*:
 * the classifier's closed vocabulary plus the tone and render-class fallbacks
 * below. Naming the set is what forces `PROGRESSIVE_VERBS` to stay complete —
 * `TONE_VERB.admin` ("Changed") once had no progressive form, so a live run of
 * mixed admin relay ops read "Changed Buzz relay ops · step 2" in the
 * present tense's place.
 */
type RunVerb =
  | "Added"
  | "Archived"
  | "Captured"
  | "Changed"
  | "Checked"
  | "Compacted"
  | "Created"
  | "Deleted"
  | "Edited"
  | "Ran"
  | "Read"
  | "Removed"
  | "Searched"
  | "Sent"
  | "Unarchived"
  | "Updated"
  | "Viewed";

/**
 * Present-participle form of every verb in `RunVerb`. A live run reads as work
 * in progress ("Reviewing files") rather than as an outcome, so the verb is
 * re-tensed here instead of being looked up in a second vocabulary.
 *
 * Exhaustive by construction: adding a verb to `RunVerb` — or a new tone
 * fallback — fails to compile until its progressive form is decided here, which
 * is the guard the missing "Changed" entry slipped through.
 */
const PROGRESSIVE_VERBS = {
  Added: "Adding",
  Archived: "Archiving",
  Captured: "Capturing",
  // Reached by a live run of mixed admin relay ops, whose steps disagree on a
  // verb and so fall back to the admin tone's.
  Changed: "Changing",
  Checked: "Checking",
  Compacted: "Compacting",
  Created: "Creating",
  Deleted: "Deleting",
  Edited: "Editing",
  Ran: "Running",
  // "Reading files" would read as one open file; "Reviewing" is how the
  // transcript has always narrated a live sweep of reads.
  Read: "Reviewing",
  Removed: "Removing",
  Searched: "Searching",
  Sent: "Sending",
  Unarchived: "Unarchiving",
  Updated: "Updating",
  Viewed: "Viewing",
} satisfies Record<RunVerb, string>;

/** Render classes in descending salience: writes and speech before reads. */
const RENDER_CLASS_SALIENCE = {
  "file-edit": 0,
  message: 1,
  "relay-op": 2,
  shell: 3,
  plan: 4,
  image: 5,
  generic: 6,
  error: 7,
  "file-read": 8,
  "skill-read": 9,
  // Ineligible for chaining; ranked last so a recovered odd class never
  // outranks real work.
  permission: 10,
  "raw-rail": 11,
  suppressed: 12,
  thought: 13,
  status: 14,
} satisfies Record<AgentActivityRenderClass, number>;

/** Tones in descending salience, so admin/write work outranks reads. */
const TONE_SALIENCE = {
  admin: 0,
  write: 1,
  neutral: 2,
  read: 3,
} satisfies Record<AgentActivityTone, number>;

/**
 * The kind of work a step represents: its render class plus the classifier's
 * tone. Tone is part of the identity so a mixed run of Buzz relay ops
 * headlines as reading, writing, or administering rather than flattening every
 * relay op into one generic phrase.
 */
export type ToolRunKind = {
  renderClass: AgentActivityRenderClass;
  tone: AgentActivityTone;
};

/**
 * Verb floor per tone, used when the steps that define a run's headline do not
 * agree on one verb (a mixed sweep of relay writes, say). The classifier's own
 * tone→verb fallback uses the same "Read"/"Updated" split; "Changed" is the
 * admin equivalent, which only a run needs because a single admin step always
 * has a specific verb ("Created", "Removed") of its own.
 */
const TONE_VERB: Record<AgentActivityTone, RunVerb | null> = {
  admin: "Changed",
  write: "Updated",
  read: "Read",
  neutral: null,
};

/**
 * Aggregate lifecycle of a run.
 *
 * `running` deliberately wins over `error` for the *phase* (the spec's
 * "running spinner while any step executes"). A failure is never masked by
 * that: `hasError` is tracked separately, a live run is expanded by default,
 * and the failing step carries its own highlight — so an error inside a
 * still-running chain is visible in the body while the header keeps reporting
 * that work is ongoing.
 */
export type ToolRunPhase = "running" | "done" | "error";

/** Verb/object/outcome headline for a run, pre-split for `ActivityRowLabel`. */
export type ToolRunHeadline = {
  verb: string;
  /** Object phrase; carries the count for a homogeneous run ("3 files"). */
  object: string | null;
  /** Trailing clause — "4 steps" while collapsed, "step 3" while live. */
  detail: string | null;
};

export type ToolRunAggregate = {
  phase: ToolRunPhase;
  hasError: boolean;
  /** Number of steps in the run. */
  count: number;
  /** 1-based position of the step currently executing, if any. */
  activeStep: number | null;
  /** Count of steps that failed. */
  errorCount: number;
};

/**
 * Whether a transcript item may join a tool-chain run.
 *
 * Eligibility is decided on the step's *recovered* class, not on whatever class
 * its descriptor happens to carry. `classifyTool` flattens every failed step to
 * render class `error`, which would otherwise let a failure launder an excluded
 * class into an eligible one: a failed `stop` hook still carries `groupKey`
 * `suppressed:stop-hook` but reports as `error`, and admitting that would fold
 * the ambient safety net into a collapsed card — precisely what the exclusions
 * above exist to prevent. `toolRunKind` already undoes that flattening for the
 * headline, so deciding eligibility through it keeps one recovery path rather
 * than two that can disagree.
 */
export function isToolRunEligible(item: TranscriptItem): boolean {
  if (item.type !== "tool") return false;
  return CHAIN_ELIGIBLE_RENDER_CLASSES[toolRunKind(item).renderClass] === true;
}

/** Effective render class for a tool item (descriptor fallback included). */
export function toolRunRenderClass(item: ToolItem): AgentActivityRenderClass {
  const descriptor = item.descriptor ?? classifyToolItem(item);
  return item.renderClass ?? descriptor.renderClass;
}

/**
 * Semantic identity used to decide whether a run is homogeneous. Falls back to
 * the render class when the classifier supplied no `groupKey`.
 */
export function toolRunGroupKey(item: ToolItem): string {
  const descriptor = item.descriptor ?? classifyToolItem(item);
  return descriptor.groupKey ?? toolRunRenderClass(item);
}

/**
 * The descriptor a run reads a step's *semantics* from.
 *
 * `classifyTool` rewrites a failed step to render class `error` with a "…
 * failed" label. That is right for the step's own row, but it erases what the
 * step was doing — so a run containing one failed read would headline as
 * anonymous tool work. When a descriptor arrives already flattened this
 * re-classifies the step as if it had succeeded, recovering its original class,
 * tone, and verb. Nothing is hidden by that: the failure is carried by the
 * aggregate status glyph and by the failing step's own highlighted row.
 */
function toolRunDescriptor(item: ToolItem): AgentActivityDescriptor {
  const descriptor = item.descriptor ?? classifyToolItem(item);
  if (descriptor.renderClass !== "error") return descriptor;

  const recovered = classifyTool({
    title: item.title,
    toolName: item.toolName,
    buzzToolName: item.buzzToolName,
    args: item.args,
    result: item.result,
    isError: false,
  });
  // Some steps are genuinely nothing but a failure (no recoverable tool
  // identity); those keep reporting as an error rather than being guessed at.
  return recovered.renderClass === "error" ? descriptor : recovered;
}

/** Render class plus tone: the kind of work one step represents. */
export function toolRunKind(item: ToolItem): ToolRunKind {
  const descriptor = toolRunDescriptor(item);
  // `item.renderClass` is canonical except when it too was flattened to
  // "error", in which case the recovered descriptor's class is the honest one.
  const renderClass =
    item.renderClass && item.renderClass !== "error"
      ? item.renderClass
      : descriptor.renderClass;
  return { renderClass, tone: descriptor.tone ?? "neutral" };
}

function sameToolRunKind(left: ToolRunKind, right: ToolRunKind): boolean {
  return left.renderClass === right.renderClass && left.tone === right.tone;
}

function isToolStepRunning(item: ToolItem): boolean {
  return item.status === "executing" || item.status === "pending";
}

function isToolStepFailed(item: ToolItem): boolean {
  return item.isError || item.status === "failed";
}

/** Aggregate status across a run's steps. */
export function summarizeToolRunStatus(items: ToolItem[]): ToolRunAggregate {
  let activeStep: number | null = null;
  let errorCount = 0;

  items.forEach((item, index) => {
    if (activeStep === null && isToolStepRunning(item)) {
      activeStep = index + 1;
    }
    if (isToolStepFailed(item)) {
      errorCount += 1;
    }
  });

  const hasError = errorCount > 0;
  const phase: ToolRunPhase =
    activeStep !== null ? "running" : hasError ? "error" : "done";

  return { phase, hasError, count: items.length, activeStep, errorCount };
}

/**
 * Past-tense label for a run whose steps all share one semantic group key.
 * These are the specific, countable sentences ("Read 3 files") the transcript
 * has always used; keep them verbatim so a homogeneous run reads no differently
 * than it did before chains existed.
 */
function homogeneousHeadline(items: ToolItem[]): ToolRunHeadline {
  const count = items.length;
  const kind = toolRunKind(items[0]);
  const phrase = RENDER_CLASS_PHRASE[kind.renderClass];

  // Work the classifier could only name generically has no honest collective
  // noun, so it keeps the classifier's own label ("Queried registry ×2")
  // instead of counting anonymous "tool calls".
  if (!phrase.countable) {
    return {
      verb: toolRunDescriptor(items[0]).label,
      object: `×${count}`,
      detail: null,
    };
  }

  return {
    verb: runVerb(items, kind),
    object: `${count} ${pluralize(phrase.noun, count)}`,
    detail: null,
  };
}

function pluralize(noun: string, count: number): string {
  return count === 1 ? noun : `${noun}s`;
}

/**
 * Present-participle form of a run's verb, for a live header.
 *
 * Verbs this module chooses are `RunVerb` and always have an entry. A verb that
 * came straight from a descriptor is only typed `string` — the classifier could
 * grow one this table has not seen — so an unknown verb degrades to itself
 * rather than to something wrong.
 */
function progressiveVerb(verb: string): string {
  return PROGRESSIVE_VERBS[verb as RunVerb] ?? verb;
}

/**
 * The verb a run headlines with.
 *
 * Vocabulary comes from the classifier: each step's descriptor already carries
 * an `action.verb` chosen from the tool and its tone, so a run of relay reads
 * says "Read" and a run of relay writes says "Updated" without this module
 * re-deriving either. When the steps that define the headline disagree on a
 * verb (a mixed sweep of writes, say) the run falls back to the tone's verb,
 * and finally to the render class's floor.
 */
function runVerb(items: ToolItem[], kind: ToolRunKind): string {
  const verbs = new Set<string>();
  for (const item of items) {
    if (!sameToolRunKind(toolRunKind(item), kind)) continue;
    const verb = toolRunDescriptor(item).action?.verb;
    if (verb) verbs.add(verb);
  }

  if (verbs.size === 1) {
    const [only] = verbs;
    return only;
  }

  return TONE_VERB[kind.tone] ?? RENDER_CLASS_PHRASE[kind.renderClass].verb;
}

/**
 * The kind of work that dominates a run. Ties break toward the more salient
 * render class, then the more salient tone — so a run that wrote as much as it
 * read headlines as writing ("failures rise; reads recede").
 */
export function dominantToolRunKind(items: ToolItem[]): ToolRunKind {
  const counts = new Map<string, { count: number; kind: ToolRunKind }>();
  for (const item of items) {
    const kind = toolRunKind(item);
    const key = `${kind.renderClass}:${kind.tone}`;
    const entry = counts.get(key);
    if (entry) {
      entry.count += 1;
    } else {
      counts.set(key, { count: 1, kind });
    }
  }

  let dominant: { count: number; kind: ToolRunKind } | null = null;
  for (const entry of counts.values()) {
    if (dominant === null || outranks(entry, dominant)) {
      dominant = entry;
    }
  }

  return dominant?.kind ?? { renderClass: "generic", tone: "neutral" };
}

function outranks(
  candidate: { count: number; kind: ToolRunKind },
  incumbent: { count: number; kind: ToolRunKind },
): boolean {
  if (candidate.count !== incumbent.count) {
    return candidate.count > incumbent.count;
  }
  const candidateClass = RENDER_CLASS_SALIENCE[candidate.kind.renderClass];
  const incumbentClass = RENDER_CLASS_SALIENCE[incumbent.kind.renderClass];
  if (candidateClass !== incumbentClass) {
    return candidateClass < incumbentClass;
  }
  return (
    TONE_SALIENCE[candidate.kind.tone] < TONE_SALIENCE[incumbent.kind.tone]
  );
}

/**
 * Headline for a run: verb, object, and an optional trailing clause.
 *
 * This is the run's ONE headline derivation — the collapsed header and the
 * live header are the same sentence in two tenses, and both read their
 * vocabulary from the classifier's descriptors rather than from a parallel
 * taxonomy.
 *
 * A live run reads as active and reports how far it has got
 * ("Reviewing files · step 3"). A settled run reads as an outcome: homogeneous
 * runs keep their specific countable sentence ("Read 4 files"), heterogeneous
 * runs name the dominant kind of work and carry the step count in the detail
 * clause ("Read files · 6 steps") rather than pretending to be one thing.
 */
export function summarizeToolRunHeadline(
  items: ToolItem[],
  aggregate: ToolRunAggregate,
): ToolRunHeadline {
  if (items.length === 0) {
    return { verb: "Working…", object: null, detail: null };
  }

  const groupKey = toolRunGroupKey(items[0]);
  const homogeneous = items.every((item) => toolRunGroupKey(item) === groupKey);

  if (aggregate.phase === "running") {
    const kind = homogeneous
      ? toolRunKind(items[0])
      : dominantToolRunKind(items);
    const verb = runVerb(items, kind);
    return {
      verb: progressiveVerb(verb),
      // A live run has no final count to report, so it names the kind of work
      // and lets the detail clause carry progress.
      object: pluralize(RENDER_CLASS_PHRASE[kind.renderClass].noun, 2),
      detail:
        aggregate.activeStep === null ? null : `step ${aggregate.activeStep}`,
    };
  }

  if (homogeneous) {
    return homogeneousHeadline(items);
  }

  const kind = dominantToolRunKind(items);
  return {
    verb: runVerb(items, kind),
    object: pluralize(RENDER_CLASS_PHRASE[kind.renderClass].noun, 2),
    detail: `${items.length} steps`,
  };
}

/** Earliest start instant across a run, in epoch ms. */
export function toolRunStartedAtMs(items: ToolItem[]): number | null {
  let earliest: number | null = null;
  for (const item of items) {
    const parsed = Date.parse(item.startedAt || item.timestamp);
    if (Number.isNaN(parsed)) continue;
    if (earliest === null || parsed < earliest) earliest = parsed;
  }
  return earliest;
}

/**
 * Latest completion instant across a run, in epoch ms — null unless every step
 * has settled, so a partially-finished run never reports a final duration.
 */
export function toolRunCompletedAtMs(items: ToolItem[]): number | null {
  let latest: number | null = null;
  for (const item of items) {
    if (!item.completedAt) return null;
    const parsed = Date.parse(item.completedAt);
    if (Number.isNaN(parsed)) return null;
    if (latest === null || parsed > latest) latest = parsed;
  }
  return latest;
}

/**
 * Wall-clock span of a run in ms: first start to last completion once settled,
 * or first start to `now` while still executing. Null when unmeasurable.
 */
export function toolRunElapsedMs(
  items: ToolItem[],
  now: number,
): number | null {
  const startedAt = toolRunStartedAtMs(items);
  if (startedAt === null) return null;
  const completedAt = toolRunCompletedAtMs(items);
  const end = completedAt ?? now;
  const elapsed = end - startedAt;
  return elapsed < 0 ? null : elapsed;
}
