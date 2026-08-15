import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_JOB_ACCEPTED,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_REQUEST,
  KIND_JOB_RESULT,
} from "@/shared/constants/kinds";

const JOB_KIND: Record<number, true> = {
  [KIND_JOB_REQUEST]: true,
  [KIND_JOB_ACCEPTED]: true,
  [KIND_JOB_PROGRESS]: true,
  [KIND_JOB_RESULT]: true,
  [KIND_JOB_CANCEL]: true,
  [KIND_JOB_ERROR]: true,
};
const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

type JsonRecord = Record<string, unknown>;

export type AgentJobState =
  | "requested"
  | "accepted"
  | "running"
  | "cancelling"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "lost";

export type AgentJobArtifact = {
  name: string;
  uri: string;
  sha256?: string;
};

export type AgentJobView = {
  jobId: string;
  requestEventId: string;
  targetPubkey: string;
  sourceEventId: string | null;
  channelId: string;
  state: AgentJobState;
  summary: string;
  attempt: number | null;
  progressSeq: number | null;
  requestedAt: number;
  startedAt: number | null;
  finishedAt: number | null;
  exitCode: number | null;
  errorCode: string | null;
  artifacts: AgentJobArtifact[];
  publicationFailed: boolean;
  eventIds: string[];
};

export type AgentJobProjection = {
  viewsByRepresentativeEventId: ReadonlyMap<string, AgentJobView>;
  collapsedEventIds: ReadonlySet<string>;
};

type ParsedLifecycle = {
  event: RelayEvent;
  attempt: number | null;
  seq: number | null;
  state: AgentJobState;
  summary: string | null;
  acceptedAt: number | null;
  finishedAt: number | null;
  exitCode: number | null;
  errorCode: string | null;
  artifacts: AgentJobArtifact[];
};

export function isAgentJobEvent(event: Pick<RelayEvent, "kind">): boolean {
  return JOB_KIND[event.kind] === true;
}

/**
 * Collapse only complete, internally linked job chains. Any malformed or lone
 * event is deliberately omitted from the projection so the timeline keeps the
 * original signed events instead of fabricating a job state.
 */
export function reduceAgentJobEvents(
  events: readonly RelayEvent[],
): AgentJobProjection {
  const groups = new Map<string, RelayEvent[]>();
  const conflictedJobs = new Set<string>();
  const eventById = new Map<string, RelayEvent>();
  const jobByEventId = new Map<string, string>();

  for (const event of events) {
    if (!isAgentJobEvent(event)) continue;
    const jobId = exactlyOneTag(event.tags, "job");
    if (!jobId || !UUID_RE.test(jobId)) continue;
    const normalizedJobId = jobId.toLowerCase();
    const prior = eventById.get(event.id);
    if (prior) {
      if (!sameEvent(prior, event)) {
        conflictedJobs.add(normalizedJobId);
        const priorJobId = jobByEventId.get(event.id);
        if (priorJobId) conflictedJobs.add(priorJobId);
      }
      continue;
    }
    eventById.set(event.id, event);
    jobByEventId.set(event.id, normalizedJobId);
    const group = groups.get(normalizedJobId) ?? [];
    group.push(event);
    groups.set(normalizedJobId, group);
  }
  const viewsByRepresentativeEventId = new Map<string, AgentJobView>();
  const collapsedEventIds = new Set<string>();

  for (const [jobId, group] of groups) {
    if (conflictedJobs.has(jobId)) continue;
    const view = reduceOneJob(jobId, group);
    if (!view) continue;
    viewsByRepresentativeEventId.set(view.requestEventId, view);
    for (const eventId of view.eventIds) {
      if (eventId !== view.requestEventId) collapsedEventIds.add(eventId);
    }
  }

  return { viewsByRepresentativeEventId, collapsedEventIds };
}

function reduceOneJob(
  jobId: string,
  events: readonly RelayEvent[],
): AgentJobView | null {
  const requests = events.filter((event) => event.kind === KIND_JOB_REQUEST);
  // A request alone is intentionally raw: there is no signed acceptance or
  // failure yet from which to derive a trustworthy lifecycle view.
  if (requests.length !== 1 || events.length < 2) return null;

  const request = requests[0];
  const channelId = exactlyOneTag(request.tags, "h");
  const targetPubkey = exactlyOneTag(request.tags, "p")?.toLowerCase();
  const sourceTags = request.tags.filter((tag) => tag[0] === "e");
  const sourceEventId = sourceTags[0]?.[1] ?? null;
  const requestPayload = parseRequest(request.content);
  if (
    !channelId ||
    !targetPubkey ||
    sourceTags.length > 1 ||
    (sourceTags.length === 1 && !sourceEventId) ||
    !requestPayload
  ) {
    return null;
  }

  const lifecycle: ParsedLifecycle[] = [];
  for (const event of events) {
    if (event === request) continue;
    if (
      exactlyOneTag(event.tags, "job")?.toLowerCase() !== jobId ||
      exactlyOneTag(event.tags, "h") !== channelId ||
      exactlyOneTag(event.tags, "e") !== request.id ||
      exactlyOneTag(event.tags, "p") == null
    ) {
      return null;
    }
    const parsed = parseLifecycle(event, jobId);
    if (!parsed) return null;
    if (
      event.kind !== KIND_JOB_CANCEL &&
      event.pubkey.toLowerCase() !== targetPubkey
    ) {
      return null;
    }
    lifecycle.push(parsed);
  }

  const accepted = lifecycle.filter(
    (item) => item.event.kind === KIND_JOB_ACCEPTED,
  );
  const progress = lifecycle.filter(
    (item) => item.event.kind === KIND_JOB_PROGRESS,
  );
  const terminals = lifecycle.filter(
    (item) =>
      item.event.kind === KIND_JOB_RESULT || item.event.kind === KIND_JOB_ERROR,
  );
  if (accepted.length > 1 || terminals.length > 1) return null;
  if (
    lifecycle.some(
      (item) =>
        item.event.kind !== KIND_JOB_CANCEL &&
        item.event.kind !== KIND_JOB_ERROR &&
        accepted.length === 0,
    )
  ) {
    return null;
  }

  const attempts = lifecycle
    .map((item) => item.attempt)
    .filter((attempt): attempt is number => attempt != null);
  if (new Set(attempts).size > 1) return null;

  const progressSeqs = progress.map((item) => item.seq);
  if (
    progressSeqs.some((seq) => seq == null) ||
    new Set(progressSeqs).size !== progressSeqs.length
  ) {
    return null;
  }

  const terminal = terminals[0] ?? null;
  if (
    terminal &&
    progress.some((item) => item.event.created_at > terminal.event.created_at)
  ) {
    return null;
  }

  const latestProgress = [...progress].sort(
    (left, right) =>
      (right.seq ?? -1) - (left.seq ?? -1) ||
      right.event.created_at - left.event.created_at ||
      right.event.id.localeCompare(left.event.id),
  )[0];
  const cancel = lifecycle
    .filter((item) => item.event.kind === KIND_JOB_CANCEL)
    .sort(
      (left, right) =>
        right.event.created_at - left.event.created_at ||
        right.event.id.localeCompare(left.event.id),
    )[0];
  const current = terminal ?? latestProgress ?? accepted[0] ?? cancel;
  if (!current) return null;

  const state = terminal
    ? terminal.state
    : cancel &&
        (!latestProgress ||
          cancel.event.created_at >= latestProgress.event.created_at)
      ? "cancelling"
      : current.state;
  const summary =
    terminal?.summary ?? latestProgress?.summary ?? requestPayload.summary;
  const artifactSource = terminal ?? latestProgress;
  const startedAt =
    accepted[0]?.acceptedAt ?? accepted[0]?.event.created_at ?? null;

  return {
    jobId,
    requestEventId: request.id,
    targetPubkey,
    sourceEventId,
    channelId,
    state,
    summary,
    attempt: attempts[0] ?? null,
    progressSeq: latestProgress?.seq ?? null,
    requestedAt: request.created_at,
    startedAt,
    finishedAt: terminal?.finishedAt ?? null,
    exitCode: terminal?.exitCode ?? null,
    errorCode: terminal?.errorCode ?? null,
    artifacts: artifactSource?.artifacts ?? [],
    publicationFailed: false,
    eventIds: [request, ...lifecycle.map((item) => item.event)]
      .sort(
        (left, right) =>
          left.created_at - right.created_at || left.id.localeCompare(right.id),
      )
      .map((event) => event.id),
  };
}

function parseRequest(content: string): { summary: string } | null {
  const value = parseRecord(content);
  if (
    !value ||
    !hasOnlyKeys(value, ["schema", "driver", "argv", "cwd", "summary"])
  ) {
    return null;
  }
  if (
    value.schema !== 1 ||
    value.driver !== "lh" ||
    !Array.isArray(value.argv) ||
    !value.argv.every((item) => typeof item === "string") ||
    typeof value.cwd !== "string" ||
    typeof value.summary !== "string"
  ) {
    return null;
  }
  return { summary: value.summary };
}

function parseLifecycle(
  event: RelayEvent,
  jobId: string,
): ParsedLifecycle | null {
  const value = parseRecord(event.content);
  if (value?.schema !== 1 || value.job !== jobId) return null;

  if (event.kind === KIND_JOB_CANCEL) {
    if (
      !hasOnlyKeys(value, ["schema", "job", "reason"]) ||
      typeof value.reason !== "string"
    ) {
      return null;
    }
    return lifecycle(event, null, null, "cancelling");
  }

  const attempt = positiveInteger(value.attempt);
  if (attempt == null) return null;

  if (event.kind === KIND_JOB_ACCEPTED) {
    if (
      !hasOnlyKeys(value, [
        "schema",
        "job",
        "attempt",
        "state",
        "accepted_at",
      ]) ||
      value.state !== "accepted" ||
      typeof value.accepted_at !== "string"
    ) {
      return null;
    }
    const acceptedAt = parseTimestamp(value.accepted_at);
    if (acceptedAt == null) return null;
    return lifecycle(event, attempt, null, "accepted", {
      acceptedAt,
    });
  }

  if (event.kind === KIND_JOB_PROGRESS) {
    const seq = nonNegativeInteger(value.seq);
    const state = value.state;
    if (
      !hasOnlyKeys(value, [
        "schema",
        "job",
        "attempt",
        "seq",
        "state",
        "summary",
        "artifacts",
      ]) ||
      seq == null ||
      (state !== "running" && state !== "cancelling") ||
      typeof value.summary !== "string"
    ) {
      return null;
    }
    const artifacts = parseArtifacts(value.artifacts);
    if (!artifacts) return null;
    const seqTag = exactlyOneTag(event.tags, "seq");
    if (seqTag !== String(seq)) return null;
    return lifecycle(event, attempt, seq, state, {
      summary: value.summary,
      artifacts,
    });
  }

  if (event.kind === KIND_JOB_RESULT) {
    if (
      !hasOnlyKeys(value, [
        "schema",
        "job",
        "attempt",
        "state",
        "exit_code",
        "summary",
        "artifacts",
        "finished_at",
      ]) ||
      value.state !== "succeeded" ||
      !Number.isInteger(value.exit_code) ||
      typeof value.summary !== "string" ||
      typeof value.finished_at !== "string"
    ) {
      return null;
    }
    const artifacts = parseArtifacts(value.artifacts);
    if (!artifacts) return null;
    const finishedAt = parseTimestamp(value.finished_at);
    if (finishedAt == null) return null;
    return lifecycle(event, attempt, null, "succeeded", {
      summary: value.summary,
      artifacts,
      exitCode: value.exit_code as number,
      finishedAt,
    });
  }

  if (event.kind === KIND_JOB_ERROR) {
    const state = value.state;
    if (
      !hasOnlyKeys(value, [
        "schema",
        "job",
        "attempt",
        "state",
        "code",
        "summary",
        "retryable",
        "artifacts",
        "finished_at",
      ]) ||
      (state !== "failed" && state !== "cancelled" && state !== "lost") ||
      typeof value.code !== "string" ||
      typeof value.summary !== "string" ||
      typeof value.retryable !== "boolean" ||
      typeof value.finished_at !== "string"
    ) {
      return null;
    }
    const artifacts = parseArtifacts(value.artifacts);
    if (!artifacts) return null;
    const finishedAt = parseTimestamp(value.finished_at);
    if (finishedAt == null) return null;
    return lifecycle(event, attempt, null, state, {
      summary: value.summary,
      artifacts,
      errorCode: value.code,
      finishedAt,
    });
  }

  return null;
}

function lifecycle(
  event: RelayEvent,
  attempt: number | null,
  seq: number | null,
  state: AgentJobState,
  overrides: Partial<ParsedLifecycle> = {},
): ParsedLifecycle {
  return {
    event,
    attempt,
    seq,
    state,
    summary: null,
    acceptedAt: null,
    finishedAt: null,
    exitCode: null,
    errorCode: null,
    artifacts: [],
    ...overrides,
  };
}

function parseArtifacts(value: unknown): AgentJobArtifact[] | null {
  if (!Array.isArray(value)) return null;
  const artifacts: AgentJobArtifact[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object" || Array.isArray(item)) return null;
    const record = item as JsonRecord;
    if (
      !hasOnlyKeys(record, ["name", "uri", "sha256"]) ||
      typeof record.name !== "string" ||
      typeof record.uri !== "string" ||
      !/^(https?:\/\/|nostr:)/.test(record.uri)
    ) {
      return null;
    }
    const sha256 = record.sha256;
    if (
      sha256 != null &&
      (typeof sha256 !== "string" || !/^[0-9a-f]{64}$/.test(sha256))
    ) {
      return null;
    }
    artifacts.push({
      name: record.name,
      uri: record.uri,
      ...(typeof sha256 === "string" ? { sha256 } : {}),
    });
  }
  return artifacts;
}

function parseRecord(content: string): JsonRecord | null {
  try {
    const value: unknown = JSON.parse(content);
    return value != null && typeof value === "object" && !Array.isArray(value)
      ? (value as JsonRecord)
      : null;
  } catch {
    return null;
  }
}

function hasOnlyKeys(value: JsonRecord, allowed: readonly string[]): boolean {
  return Object.keys(value).every((key) => allowed.includes(key));
}

function exactlyOneTag(tags: readonly string[][], name: string): string | null {
  let value: string | null = null;
  for (const tag of tags) {
    if (tag[0] !== name) continue;
    if (value != null || typeof tag[1] !== "string") return null;
    value = tag[1];
  }
  return value;
}

function positiveInteger(value: unknown): number | null {
  return Number.isInteger(value) && (value as number) > 0
    ? (value as number)
    : null;
}

function nonNegativeInteger(value: unknown): number | null {
  return Number.isInteger(value) && (value as number) >= 0
    ? (value as number)
    : null;
}

function parseTimestamp(value: string): number | null {
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? Math.floor(parsed / 1_000) : null;
}

function sameEvent(left: RelayEvent, right: RelayEvent): boolean {
  return (
    left.pubkey === right.pubkey &&
    left.created_at === right.created_at &&
    left.kind === right.kind &&
    left.content === right.content &&
    left.sig === right.sig &&
    JSON.stringify(left.tags) === JSON.stringify(right.tags)
  );
}
