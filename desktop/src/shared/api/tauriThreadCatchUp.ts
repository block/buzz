import { invokeTauri } from "@/shared/api/tauri";

/**
 * Typed IPC for the desktop thread recap ("Catch me up"). Wire shapes are
 * camelCase and mutations take a single `input` argument, matching the
 * native commands in `src-tauri/src/commands/thread_catch_up*.rs`.
 *
 * Every response is validated before it reaches the UI: the recap is rendered
 * from model output that native has already checked, but the client still
 * refuses malformed shapes rather than inventing links or categories.
 */

/** Identity of one recap request, frozen when the person asks for it. */
export type CatchUpScope = {
  /** Captured active relay; native normalizes ws/http before matching. */
  relayUrl: string;
  signerPubkey: string;
  channelId: string;
  rootEventId: string;
};

/** Explicit local-model setup, kept separate from global agent defaults. */
export type CatchUpSettings = {
  baseUrl: string;
  model: string;
  /** The person confirmed this endpoint runs the model on this computer. */
  confirmedLocal: boolean;
};

export type CatchUpPointCategory = "decision" | "question";

export type CatchUpPoint = {
  /** Stable within one immutable recap. */
  id: string;
  category: CatchUpPointCategory;
  text: string;
  sourceIds: string[];
};

export type CatchUpCoverage = {
  messageCount: number;
  inputMessageCount: number;
  truncated: boolean;
  warnings: string[];
};

export type CatchUpRecap = {
  version: 1;
  channelId: string;
  rootEventId: string;
  /** Unix seconds. */
  since: number;
  until: number;
  points: CatchUpPoint[];
  /** Validated message ids in the model input, including every cited id. */
  sourceIds: string[];
  coverage: CatchUpCoverage;
  /**
   * Source id → SHA-256 of native's canonical projection of that source's
   * content when the recap was generated. Opaque to the client: preserved
   * exactly and sent back unchanged on share, where native rejects the share
   * if a cited source changed. Absent on recaps from older generators.
   */
  sourceRevisions?: Record<string, string>;
};

export type CatchUpCorrection = {
  eventId: string;
  pointId: string;
  text: string;
  /** Derived from the signed revision, never from payload content. */
  authorPubkey: string;
  createdAt: number;
};

export type SharedCatchUp = {
  artifactId: string;
  /** Latest signed revision; the `prev` for the next correction. */
  eventId: string;
  authorPubkey: string;
  createdAt: number;
  recap: CatchUpRecap;
  corrections: CatchUpCorrection[];
  historyComplete: boolean;
};

export type GenerateCatchUpInput = {
  requestId: string;
  scope: CatchUpScope;
  since: number;
  until: number;
};

export type CorrectCatchUpInput = {
  scope: CatchUpScope;
  artifactId: string;
  prevEventId: string;
  pointId: string;
  text: string;
};

const HEX64_RE = /^[a-f0-9]{64}$/;
const MAX_POINTS = 12;
const MAX_POINT_TEXT_CHARS = 2_000;
const MAX_WARNINGS = 20;
const MAX_SOURCE_IDS = 400;

function fail(field: string): never {
  throw new Error(`Thread recap response was malformed (${field}).`);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function readString(value: unknown, field: string, maxChars = 10_000) {
  if (typeof value !== "string" || value.length > maxChars) fail(field);
  return value;
}

function readSeconds(value: unknown, field: string) {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    fail(field);
  }
  return value;
}

function readEventId(value: unknown, field: string) {
  const id = readString(value, field, 64).toLowerCase();
  if (!HEX64_RE.test(id)) fail(field);
  return id;
}

function readEventIds(value: unknown, field: string) {
  if (!Array.isArray(value)) fail(field);
  return value.map((id) => readEventId(id, field));
}

/** Validate a recap from native; cited ids must belong to its own input. */
export function parseCatchUpRecap(value: unknown): CatchUpRecap {
  if (!isRecord(value) || value.version !== 1) fail("version");
  const sourceIds = readEventIds(value.sourceIds, "sourceIds");
  if (sourceIds.length > MAX_SOURCE_IDS) fail("sourceIds");
  const sourceSet = new Set(sourceIds);
  if (!Array.isArray(value.points) || value.points.length > MAX_POINTS) {
    fail("points");
  }
  const pointIds = new Set<string>();
  const points = value.points.map((raw): CatchUpPoint => {
    if (!isRecord(raw)) fail("point");
    const id = readString(raw.id, "point.id", 64);
    if (id.length === 0 || pointIds.has(id)) fail("point.id");
    pointIds.add(id);
    if (raw.category !== "decision" && raw.category !== "question") {
      fail("point.category");
    }
    const text = readString(raw.text, "point.text", MAX_POINT_TEXT_CHARS);
    const pointSources = readEventIds(raw.sourceIds, "point.sourceIds");
    if (
      text.trim().length === 0 ||
      pointSources.length === 0 ||
      pointSources.some((id) => !sourceSet.has(id))
    ) {
      fail("point.sourceIds");
    }
    return { id, category: raw.category, text, sourceIds: pointSources };
  });
  const coverage = value.coverage;
  if (!isRecord(coverage) || typeof coverage.truncated !== "boolean") {
    fail("coverage");
  }
  if (
    !Array.isArray(coverage.warnings) ||
    coverage.warnings.length > MAX_WARNINGS
  ) {
    fail("coverage.warnings");
  }
  const since = readSeconds(value.since, "since");
  const until = readSeconds(value.until, "until");
  if (until < since) fail("until");
  const sourceRevisions = readSourceRevisions(
    value.sourceRevisions,
    new Set(points.flatMap((point) => point.sourceIds)),
    points,
  );
  return {
    version: 1,
    channelId: readString(value.channelId, "channelId", 128),
    rootEventId: readEventId(value.rootEventId, "rootEventId"),
    since,
    until,
    points,
    sourceIds,
    coverage: {
      messageCount: readSeconds(coverage.messageCount, "messageCount"),
      inputMessageCount: readSeconds(
        coverage.inputMessageCount,
        "inputMessageCount",
      ),
      truncated: coverage.truncated,
      warnings: coverage.warnings.map((warning) =>
        readString(warning, "coverage.warning", 500),
      ),
    },
    ...(sourceRevisions ? { sourceRevisions } : {}),
  };
}

/**
 * Validate native's source revision map without normalizing it: keys and
 * values must already be lowercase hex so the map round-trips byte for byte.
 */
function readSourceRevisions(
  value: unknown,
  sourceSet: ReadonlySet<string>,
  points: readonly CatchUpPoint[],
): Record<string, string> | undefined {
  if (value === undefined) return undefined;
  if (!isRecord(value)) fail("sourceRevisions");
  const entries = Object.entries(value);
  if (entries.length > MAX_SOURCE_IDS) fail("sourceRevisions");
  const revisions: Record<string, string> = {};
  for (const [id, hash] of entries) {
    if (
      !HEX64_RE.test(id) ||
      !sourceSet.has(id) ||
      typeof hash !== "string" ||
      !HEX64_RE.test(hash)
    ) {
      fail("sourceRevisions");
    }
    revisions[id] = hash;
  }
  if (
    points.some((point) =>
      point.sourceIds.some((id) => !Object.hasOwn(revisions, id)),
    )
  ) {
    fail("sourceRevisions");
  }
  return revisions;
}

function parseCorrection(value: unknown): CatchUpCorrection {
  if (!isRecord(value)) fail("correction");
  return {
    eventId: readEventId(value.eventId, "correction.eventId"),
    pointId: readString(value.pointId, "correction.pointId", 64),
    text: readString(value.text, "correction.text", MAX_POINT_TEXT_CHARS),
    authorPubkey: readEventId(value.authorPubkey, "correction.authorPubkey"),
    createdAt: readSeconds(value.createdAt, "correction.createdAt"),
  };
}

/** Validate one shared recap; corrections must target points it contains. */
export function parseSharedCatchUp(value: unknown): SharedCatchUp {
  if (!isRecord(value)) fail("shared");
  const recap = parseCatchUpRecap(value.recap);
  const pointIds = new Set(recap.points.map((point) => point.id));
  if (!Array.isArray(value.corrections)) fail("corrections");
  const corrections = value.corrections.map(parseCorrection);
  if (corrections.some((correction) => !pointIds.has(correction.pointId))) {
    fail("correction.pointId");
  }
  if (typeof value.historyComplete !== "boolean") fail("historyComplete");
  return {
    artifactId: readString(value.artifactId, "artifactId", 256),
    eventId: readEventId(value.eventId, "eventId"),
    authorPubkey: readEventId(value.authorPubkey, "authorPubkey"),
    createdAt: readSeconds(value.createdAt, "createdAt"),
    recap,
    corrections,
    historyComplete: value.historyComplete,
  };
}

function parseSettings(value: unknown): CatchUpSettings {
  if (!isRecord(value) || typeof value.confirmedLocal !== "boolean") {
    fail("settings");
  }
  return {
    baseUrl: readString(value.baseUrl, "settings.baseUrl", 2_048),
    model: readString(value.model, "settings.model", 256),
    confirmedLocal: value.confirmedLocal,
  };
}

export async function getThreadCatchUpSettings(): Promise<CatchUpSettings> {
  return parseSettings(await invokeTauri("get_thread_catch_up_settings"));
}

export async function setThreadCatchUpSettings(
  input: CatchUpSettings,
): Promise<CatchUpSettings> {
  return parseSettings(
    await invokeTauri("set_thread_catch_up_settings", { input }),
  );
}

export async function generateThreadCatchUp(
  input: GenerateCatchUpInput,
): Promise<CatchUpRecap> {
  return parseCatchUpRecap(
    await invokeTauri("generate_thread_catch_up", { input }),
  );
}

export async function cancelThreadCatchUp(requestId: string): Promise<void> {
  await invokeTauri("cancel_thread_catch_up", { input: { requestId } });
}

export type SharedCatchUpList = {
  /** Most recently changed first; same field name as the native response. */
  recaps: SharedCatchUp[];
  /**
   * Recaps left out because native or this client could not verify or
   * interpret them; surfaced rather than silently dropped.
   */
  unreadableCount: number;
  /** False when the relay had more shared recaps than one list returns. */
  complete: boolean;
};

/** Validate the list per entry so one bad recap cannot hide the others. */
export function parseSharedCatchUpList(value: unknown): SharedCatchUpList {
  if (
    !isRecord(value) ||
    !Array.isArray(value.recaps) ||
    typeof value.complete !== "boolean"
  ) {
    fail("artifacts");
  }
  const recaps: SharedCatchUp[] = [];
  let unreadableCount = readSeconds(value.unreadableCount, "unreadableCount");
  for (const entry of value.recaps) {
    try {
      recaps.push(parseSharedCatchUp(entry));
    } catch {
      unreadableCount += 1;
    }
  }
  return { recaps, unreadableCount, complete: value.complete };
}

export async function listThreadCatchUpArtifacts(
  scope: CatchUpScope,
): Promise<SharedCatchUpList> {
  return parseSharedCatchUpList(
    await invokeTauri("list_thread_catch_up_artifacts", { input: { scope } }),
  );
}

export async function shareThreadCatchUp(input: {
  scope: CatchUpScope;
  recap: CatchUpRecap;
}): Promise<SharedCatchUp> {
  return parseSharedCatchUp(
    await invokeTauri("share_thread_catch_up", { input }),
  );
}

export async function correctThreadCatchUp(
  input: CorrectCatchUpInput,
): Promise<SharedCatchUp> {
  return parseSharedCatchUp(
    await invokeTauri("correct_thread_catch_up", { input }),
  );
}
