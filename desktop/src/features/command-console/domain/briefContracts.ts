import { hasExactKeys, isRecord, isRfc3339 } from "./validation";

export const MAX_TEXT_BYTES = 4096;
export const MAX_ARRAY_ITEMS = 64;
const MAX_LEDGER_ITEMS = 256;

export const ADVISORY_LIMITATION =
  "This Daily Command Brief is advisory only. Navigation content identifies considerations and source limitations; it does not generate executable navigation orders or make navigational decisions.";

export const BRIEF_SECTIONS = Object.freeze([
  "today",
  "operations",
  "navigation",
  "daily_routine",
  "reports",
  "planning_30_60_90",
  "decisions",
  "conflicts_and_gaps",
  "sources",
] as const);
export type BriefSection = (typeof BRIEF_SECTIONS)[number];

export const ADVISER_IDS = Object.freeze([
  "chief_of_staff",
  "operations",
  "navigation",
  "daily_routine",
  "reporting",
  "plans",
] as const);
export type AdviserId = (typeof ADVISER_IDS)[number];

const SPECIALISTS = Object.freeze([
  "operations",
  "navigation",
  "daily_routine",
  "reporting",
  "plans",
] as const);

const SOURCE_KINDS = Object.freeze([
  "rag",
  "memory",
  "calendar",
  "reminders",
  "notes",
  "file",
] as const);

const RUN_STATES = Object.freeze([
  "queued",
  "collecting_sources",
  "running_specialists",
  "consolidating",
  "persisting",
  "completed",
  "degraded",
  "cancelled",
  "failed",
] as const);
export type BriefRunState = (typeof RUN_STATES)[number];

export type CitedFinding = {
  readonly classification: "OFFICIAL";
  readonly text: string;
  readonly sourceIds: readonly string[];
};

export type SourceLedgerEntry = {
  readonly classification: "OFFICIAL";
  readonly ledgerId: string;
  readonly sourceId: string;
  readonly sourceKind: (typeof SOURCE_KINDS)[number];
  readonly collection: string;
  readonly documentId: string;
  readonly chunkId: string;
  readonly timestamp: string;
  readonly snapshotId: string;
  readonly quotedLocation: {
    readonly quote: string;
    readonly location: string;
  };
  readonly retrievedAt: string;
  readonly observedAt: string;
};

export type AdviserContribution = {
  readonly classification: "OFFICIAL";
  readonly adviser: (typeof SPECIALISTS)[number];
  readonly section: BriefSection;
  readonly findings: readonly CitedFinding[];
  readonly confidence: number;
  readonly limitations: readonly string[];
  readonly dissent: readonly string[];
  readonly proposedActions: readonly {
    readonly classification: "OFFICIAL";
    readonly actionId: string;
    readonly text: string;
    readonly approvalState: "pending";
  }[];
};

export type CommandBrief = {
  readonly version: 1;
  readonly classification: "OFFICIAL";
  readonly generatedAt: string;
  readonly runId: string;
  readonly scheduleId: string;
  readonly snapshotId: string;
  readonly sections: Readonly<Record<BriefSection, readonly CitedFinding[]>>;
  readonly degradedSections: readonly BriefSection[];
  readonly missingInformation: readonly string[];
  readonly dissent: readonly string[];
  readonly sourceLedger: readonly SourceLedgerEntry[];
  readonly sourceFreshness: {
    readonly classification: "OFFICIAL";
    readonly asOf: string;
    readonly staleSourceIds: readonly string[];
  };
  readonly contributions: readonly AdviserContribution[];
  readonly advisoryLimitation: typeof ADVISORY_LIMITATION;
};

export type PublishedCommandBrief = {
  readonly classification: "OFFICIAL";
  readonly brief: CommandBrief;
  readonly lifecycleAuditEventId: string;
  readonly publicationState: "queued" | "published";
};

export type BriefRunStatus = {
  readonly classification: "OFFICIAL";
  readonly runId: string;
  readonly scheduleId: string;
  readonly state: BriefRunState;
  readonly updatedAt: string;
  readonly degradedSections: readonly BriefSection[];
  readonly error: string | null;
};

export type BriefSchedule = {
  readonly classification: "OFFICIAL";
  readonly scheduleId: string;
  readonly enabled: boolean;
  readonly localTime: string;
  readonly timezone: string;
  readonly catchUpSameDay: boolean;
  readonly concurrency: 1 | 2;
};

export type BriefLifecycleRecord = {
  readonly classification: "OFFICIAL";
  readonly runId: string;
  readonly scheduleId: string;
  readonly state: BriefRunState;
  readonly occurredAt: string;
  readonly snapshotId: string;
  readonly previousLifecycleAuditEventId: string | null;
};

function isOneOf<T extends readonly string[]>(
  value: unknown,
  values: T,
): value is T[number] {
  return typeof value === "string" && values.includes(value);
}

function hasControlCharacter(value: string): boolean {
  for (const character of value) {
    const code = character.codePointAt(0);
    if (code !== undefined && (code <= 31 || code === 127)) return true;
  }
  return false;
}

function isBoundedText(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length > 0 &&
    value.trim() === value &&
    new TextEncoder().encode(value).byteLength <= MAX_TEXT_BYTES &&
    !hasControlCharacter(value)
  );
}

function parseTextArray(
  value: unknown,
  unique = false,
): readonly string[] | null {
  if (
    !Array.isArray(value) ||
    value.length > MAX_ARRAY_ITEMS ||
    !value.every(isBoundedText) ||
    (unique && new Set(value).size !== value.length)
  ) {
    return null;
  }
  return Object.freeze([...value]);
}

function parseFinding(
  value: unknown,
  sourceIds: ReadonlySet<string>,
): CitedFinding | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, ["classification", "text", "sourceIds"]) ||
    value.classification !== "OFFICIAL" ||
    !isBoundedText(value.text)
  ) {
    return null;
  }
  const ids = parseTextArray(value.sourceIds, true);
  if (!ids || ids.length === 0 || ids.some((id) => !sourceIds.has(id)))
    return null;
  return Object.freeze({
    classification: value.classification,
    text: value.text,
    sourceIds: Object.freeze([...ids].sort()),
  });
}

function parseFindingArray(
  value: unknown,
  sourceIds: ReadonlySet<string>,
): readonly CitedFinding[] | null {
  if (!Array.isArray(value) || value.length > MAX_ARRAY_ITEMS) return null;
  const parsed: CitedFinding[] = [];
  for (const item of value) {
    const finding = parseFinding(item, sourceIds);
    if (!finding) return null;
    parsed.push(finding);
  }
  return Object.freeze(parsed);
}

function parseSource(
  value: unknown,
  snapshotId: string,
): SourceLedgerEntry | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "ledgerId",
      "classification",
      "sourceId",
      "sourceKind",
      "collection",
      "documentId",
      "chunkId",
      "timestamp",
      "snapshotId",
      "quotedLocation",
      "retrievedAt",
      "observedAt",
    ]) ||
    value.classification !== "OFFICIAL" ||
    !isBoundedText(value.ledgerId) ||
    !isBoundedText(value.sourceId) ||
    !isOneOf(value.sourceKind, SOURCE_KINDS) ||
    !isBoundedText(value.collection) ||
    !isBoundedText(value.documentId) ||
    !isBoundedText(value.chunkId) ||
    !isRfc3339(value.timestamp) ||
    !isBoundedText(value.snapshotId) ||
    value.snapshotId !== snapshotId ||
    !isRfc3339(value.retrievedAt) ||
    !isRfc3339(value.observedAt) ||
    !isRecord(value.quotedLocation) ||
    !hasExactKeys(value.quotedLocation, ["quote", "location"]) ||
    !isBoundedText(value.quotedLocation.quote) ||
    !isBoundedText(value.quotedLocation.location)
  ) {
    return null;
  }
  return Object.freeze({
    classification: value.classification,
    ledgerId: value.ledgerId,
    sourceId: value.sourceId,
    sourceKind: value.sourceKind,
    collection: value.collection,
    documentId: value.documentId,
    chunkId: value.chunkId,
    timestamp: value.timestamp,
    snapshotId: value.snapshotId,
    quotedLocation: Object.freeze({
      quote: value.quotedLocation.quote,
      location: value.quotedLocation.location,
    }),
    retrievedAt: value.retrievedAt,
    observedAt: value.observedAt,
  });
}

function expectedSection(
  adviser: AdviserContribution["adviser"],
): BriefSection {
  return {
    operations: "operations",
    navigation: "navigation",
    daily_routine: "daily_routine",
    reporting: "reports",
    plans: "planning_30_60_90",
  }[adviser] as BriefSection;
}

function parseContribution(
  value: unknown,
  sourceIds: ReadonlySet<string>,
): AdviserContribution | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "adviser",
      "classification",
      "section",
      "findings",
      "confidence",
      "limitations",
      "dissent",
      "proposedActions",
    ]) ||
    value.classification !== "OFFICIAL" ||
    !isOneOf(value.adviser, SPECIALISTS) ||
    !isOneOf(value.section, BRIEF_SECTIONS) ||
    value.section !== expectedSection(value.adviser) ||
    typeof value.confidence !== "number" ||
    !Number.isFinite(value.confidence) ||
    value.confidence < 0 ||
    value.confidence > 1
  ) {
    return null;
  }
  const findings = parseFindingArray(value.findings, sourceIds);
  const limitations = parseTextArray(value.limitations);
  const dissent = parseTextArray(value.dissent);
  if (
    !findings ||
    !limitations ||
    !dissent ||
    !Array.isArray(value.proposedActions) ||
    value.proposedActions.length > MAX_ARRAY_ITEMS
  )
    return null;
  const proposedActions =
    [] as AdviserContribution["proposedActions"][number][];
  for (const action of value.proposedActions) {
    if (
      !isRecord(action) ||
      !hasExactKeys(action, [
        "classification",
        "actionId",
        "text",
        "approvalState",
      ]) ||
      action.classification !== "OFFICIAL" ||
      !isBoundedText(action.actionId) ||
      !isBoundedText(action.text) ||
      action.approvalState !== "pending"
    )
      return null;
    proposedActions.push(
      Object.freeze({
        classification: action.classification,
        actionId: action.actionId,
        text: action.text,
        approvalState: "pending",
      }),
    );
  }
  return Object.freeze({
    classification: value.classification,
    adviser: value.adviser,
    section: value.section,
    findings,
    confidence: value.confidence,
    limitations,
    dissent,
    proposedActions: Object.freeze(proposedActions),
  });
}

/** Parses and freezes an exact validated Daily Command Brief display contract. */
export function parseCommandBrief(value: unknown): CommandBrief | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "version",
      "classification",
      "generatedAt",
      "runId",
      "scheduleId",
      "snapshotId",
      "sections",
      "degradedSections",
      "missingInformation",
      "dissent",
      "sourceLedger",
      "sourceFreshness",
      "contributions",
      "advisoryLimitation",
    ]) ||
    value.version !== 1 ||
    value.classification !== "OFFICIAL" ||
    !isRfc3339(value.generatedAt) ||
    !isBoundedText(value.runId) ||
    !isBoundedText(value.scheduleId) ||
    !isBoundedText(value.snapshotId) ||
    value.advisoryLimitation !== ADVISORY_LIMITATION ||
    !Array.isArray(value.sourceLedger) ||
    value.sourceLedger.length > MAX_LEDGER_ITEMS ||
    !isRecord(value.sourceFreshness) ||
    !hasExactKeys(value.sourceFreshness, [
      "classification",
      "asOf",
      "staleSourceIds",
    ]) ||
    value.sourceFreshness.classification !== "OFFICIAL" ||
    !isRfc3339(value.sourceFreshness.asOf)
  )
    return null;

  const sourceLedger: SourceLedgerEntry[] = [];
  const ledgerIds = new Set<string>();
  const sourceIds = new Set<string>();
  for (const candidate of value.sourceLedger) {
    const parsed = parseSource(candidate, value.snapshotId);
    if (
      !parsed ||
      ledgerIds.has(parsed.ledgerId) ||
      sourceIds.has(parsed.sourceId)
    )
      return null;
    ledgerIds.add(parsed.ledgerId);
    sourceIds.add(parsed.sourceId);
    sourceLedger.push(parsed);
  }
  const staleSourceIds = parseTextArray(
    value.sourceFreshness.staleSourceIds,
    true,
  );
  if (!staleSourceIds || staleSourceIds.some((id) => !ledgerIds.has(id)))
    return null;

  if (
    !isRecord(value.sections) ||
    !hasExactKeys(value.sections, BRIEF_SECTIONS)
  )
    return null;
  const sections = {} as Record<BriefSection, readonly CitedFinding[]>;
  for (const section of BRIEF_SECTIONS) {
    const findings = parseFindingArray(value.sections[section], ledgerIds);
    if (!findings) return null;
    sections[section] = findings;
  }
  const degradedSections = parseSectionArray(value.degradedSections, true);
  const missingInformation = parseTextArray(value.missingInformation);
  const dissent = parseTextArray(value.dissent);
  if (
    !degradedSections ||
    !missingInformation ||
    !dissent ||
    !Array.isArray(value.contributions) ||
    value.contributions.length !== SPECIALISTS.length
  )
    return null;
  const contributions: AdviserContribution[] = [];
  const advisers = new Set<string>();
  for (const candidate of value.contributions) {
    const parsed = parseContribution(candidate, ledgerIds);
    if (!parsed || advisers.has(parsed.adviser)) return null;
    advisers.add(parsed.adviser);
    contributions.push(parsed);
  }
  if (SPECIALISTS.some((adviser) => !advisers.has(adviser))) return null;
  const specialistFindings = new Set(
    contributions.flatMap((contribution) =>
      contribution.findings.map((finding) =>
        JSON.stringify([finding.text, finding.sourceIds]),
      ),
    ),
  );
  if (
    Object.values(sections)
      .flat()
      .some(
        (finding) =>
          !specialistFindings.has(
            JSON.stringify([finding.text, finding.sourceIds]),
          ),
      )
  )
    return null;

  return Object.freeze({
    version: value.version,
    classification: value.classification,
    generatedAt: value.generatedAt,
    runId: value.runId,
    scheduleId: value.scheduleId,
    snapshotId: value.snapshotId,
    sections: Object.freeze(sections),
    degradedSections,
    missingInformation,
    dissent,
    sourceLedger: Object.freeze(sourceLedger),
    sourceFreshness: Object.freeze({
      classification: value.sourceFreshness.classification,
      asOf: value.sourceFreshness.asOf,
      staleSourceIds,
    }),
    contributions: Object.freeze(contributions),
    advisoryLimitation: ADVISORY_LIMITATION,
  });
}

function parseSectionArray(
  value: unknown,
  unique: boolean,
): readonly BriefSection[] | null {
  if (
    !Array.isArray(value) ||
    value.length > MAX_ARRAY_ITEMS ||
    !value.every((item) => isOneOf(item, BRIEF_SECTIONS))
  )
    return null;
  if (unique && new Set(value).size !== value.length) return null;
  return Object.freeze([...value]);
}

/** Parses the wrapper that receives the signed lifecycle event ID after signing. */
export function parsePublishedCommandBrief(
  value: unknown,
): PublishedCommandBrief | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "classification",
      "brief",
      "lifecycleAuditEventId",
      "publicationState",
    ]) ||
    value.classification !== "OFFICIAL" ||
    !isBoundedText(value.lifecycleAuditEventId) ||
    !isOneOf(value.publicationState, ["queued", "published"] as const)
  )
    return null;
  const brief = parseCommandBrief(value.brief);
  return brief
    ? Object.freeze({
        classification: value.classification,
        brief,
        lifecycleAuditEventId: value.lifecycleAuditEventId,
        publicationState: value.publicationState,
      })
    : null;
}

/** Parses a closed run state without accepting future or renderer-invented states. */
export function parseBriefRunState(value: unknown): BriefRunState | null {
  return isOneOf(value, RUN_STATES) ? value : null;
}

/** Parses a closed, bounded lifecycle status for one locally owned run. */
export function parseBriefRunStatus(value: unknown): BriefRunStatus | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "classification",
      "runId",
      "scheduleId",
      "state",
      "updatedAt",
      "degradedSections",
      "error",
    ]) ||
    value.classification !== "OFFICIAL" ||
    !isBoundedText(value.runId) ||
    !isBoundedText(value.scheduleId) ||
    !isOneOf(value.state, RUN_STATES) ||
    !isRfc3339(value.updatedAt) ||
    (value.error !== null && !isBoundedText(value.error))
  )
    return null;
  const degradedSections = parseSectionArray(value.degradedSections, true);
  return degradedSections
    ? Object.freeze({
        classification: value.classification,
        runId: value.runId,
        scheduleId: value.scheduleId,
        state: value.state,
        updatedAt: value.updatedAt,
        degradedSections,
        error: value.error,
      })
    : null;
}

/** Parses the local schedule with its deliberate one-or-two model concurrency bound. */
export function parseBriefSchedule(value: unknown): BriefSchedule | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "classification",
      "scheduleId",
      "enabled",
      "localTime",
      "timezone",
      "catchUpSameDay",
      "concurrency",
    ]) ||
    value.classification !== "OFFICIAL" ||
    !isBoundedText(value.scheduleId) ||
    typeof value.enabled !== "boolean" ||
    typeof value.localTime !== "string" ||
    !/^\d{2}:\d{2}$/.test(value.localTime) ||
    Number(value.localTime.slice(0, 2)) > 23 ||
    Number(value.localTime.slice(3, 5)) > 59 ||
    !isBoundedText(value.timezone) ||
    typeof value.catchUpSameDay !== "boolean" ||
    (value.concurrency !== 1 && value.concurrency !== 2)
  )
    return null;
  return Object.freeze({
    classification: value.classification,
    scheduleId: value.scheduleId,
    enabled: value.enabled,
    localTime: value.localTime,
    timezone: value.timezone,
    catchUpSameDay: value.catchUpSameDay,
    concurrency: value.concurrency,
  });
}

/** Parses an append-only lifecycle record without allowing arbitrary state names. */
export function parseBriefLifecycleRecord(
  value: unknown,
): BriefLifecycleRecord | null {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      "classification",
      "runId",
      "scheduleId",
      "state",
      "occurredAt",
      "snapshotId",
      "previousLifecycleAuditEventId",
    ]) ||
    value.classification !== "OFFICIAL" ||
    !isBoundedText(value.runId) ||
    !isBoundedText(value.scheduleId) ||
    !isOneOf(value.state, RUN_STATES) ||
    !isRfc3339(value.occurredAt) ||
    !isBoundedText(value.snapshotId) ||
    (value.previousLifecycleAuditEventId !== null &&
      !isBoundedText(value.previousLifecycleAuditEventId))
  )
    return null;
  return Object.freeze({
    classification: value.classification,
    runId: value.runId,
    scheduleId: value.scheduleId,
    state: value.state,
    occurredAt: value.occurredAt,
    snapshotId: value.snapshotId,
    previousLifecycleAuditEventId: value.previousLifecycleAuditEventId,
  });
}
