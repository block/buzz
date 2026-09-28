import {
  bestieOwnerStorageKey,
  readOwnerScopedState,
  writeOwnerScopedState,
} from "./bestieOwnerScope";

import { computeBestieJobNextDueAt } from "./bestieJobSchedule";
import type {
  BestieJob,
  BestieJobAddInput,
  BestieJobSchedule,
  BestieJobScope,
  BestieJobState,
  BestieJobUpdateInput,
} from "./bestieJobTypes";

export const BESTIE_JOB_STORAGE_PREFIX = "buzz-bestie-jobs.v1";

/** Owner+relay key — persists across Assistant agent reassignment. */
export function bestieJobStorageKey(scope: BestieJobScope): string {
  return bestieOwnerStorageKey(BESTIE_JOB_STORAGE_PREFIX, scope);
}

export const EMPTY_BESTIE_JOB_STATE: BestieJobState = Object.freeze({
  firedSlotIds: Object.freeze([]) as unknown as string[],
  jobs: Object.freeze([]) as unknown as BestieJob[],
  processedMessageIds: Object.freeze([]) as unknown as string[],
  version: 1,
});

export function emptyBestieJobState(): BestieJobState {
  return EMPTY_BESTIE_JOB_STATE;
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function parseSchedule(value: unknown): BestieJobSchedule | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.kind === "once" && isFiniteNonNegative(record.dueAt)) {
    return { dueAt: Math.floor(record.dueAt), kind: "once" };
  }
  if (record.kind === "interval" && isFiniteNonNegative(record.everySeconds)) {
    return {
      everySeconds: Math.max(1, Math.floor(record.everySeconds)),
      kind: "interval",
    };
  }
  if (
    record.kind === "daily" &&
    isFiniteNonNegative(record.hour) &&
    isFiniteNonNegative(record.minute)
  ) {
    return {
      hour: Math.floor(record.hour),
      kind: "daily",
      minute: Math.floor(record.minute),
    };
  }
  if (
    record.kind === "weekly" &&
    isFiniteNonNegative(record.weekday) &&
    isFiniteNonNegative(record.hour) &&
    isFiniteNonNegative(record.minute)
  ) {
    return {
      hour: Math.floor(record.hour),
      kind: "weekly",
      minute: Math.floor(record.minute),
      weekday: Math.floor(record.weekday),
    };
  }
  return null;
}

export function parseBestieJob(value: unknown): BestieJob | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  const schedule = parseSchedule(record.schedule);
  if (
    typeof record.id !== "string" ||
    record.id.length === 0 ||
    typeof record.title !== "string" ||
    record.title.trim().length === 0 ||
    typeof record.prompt !== "string" ||
    record.prompt.trim().length === 0 ||
    typeof record.enabled !== "boolean" ||
    !schedule ||
    !isFiniteNonNegative(record.createdAt) ||
    !isFiniteNonNegative(record.updatedAt)
  ) {
    return null;
  }
  const lastRunAt =
    record.lastRunAt == null
      ? null
      : isFiniteNonNegative(record.lastRunAt)
        ? record.lastRunAt
        : null;
  const nextDueAt =
    record.nextDueAt == null
      ? null
      : isFiniteNonNegative(record.nextDueAt)
        ? record.nextDueAt
        : null;
  const sourceMessageId =
    typeof record.sourceMessageId === "string" &&
    record.sourceMessageId.length > 0
      ? record.sourceMessageId
      : null;
  return {
    createdAt: record.createdAt,
    enabled: record.enabled,
    id: record.id,
    lastRunAt,
    nextDueAt,
    prompt: record.prompt.trim(),
    schedule,
    sourceMessageId,
    title: record.title.trim(),
    updatedAt: record.updatedAt,
  };
}

export function parseBestieJobState(value: unknown): BestieJobState | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (record.version !== 1 || !Array.isArray(record.jobs)) return null;
  const jobs: BestieJob[] = [];
  for (const entry of record.jobs) {
    const job = parseBestieJob(entry);
    if (job) jobs.push(job);
  }
  const processedMessageIds = Array.isArray(record.processedMessageIds)
    ? record.processedMessageIds.filter(
        (id): id is string => typeof id === "string" && id.length > 0,
      )
    : [];
  const firedSlotIds = Array.isArray(record.firedSlotIds)
    ? record.firedSlotIds.filter(
        (id): id is string => typeof id === "string" && id.length > 0,
      )
    : [];
  return { firedSlotIds, jobs, processedMessageIds, version: 1 };
}

function mergeBestieJobStates(
  into: BestieJobState,
  from: BestieJobState,
): BestieJobState {
  const byId = new Map(into.jobs.map((job) => [job.id, job]));
  for (const job of from.jobs) {
    const existing = byId.get(job.id);
    if (!existing || job.updatedAt >= existing.updatedAt) {
      byId.set(job.id, job);
    }
  }
  const fired = new Set([...into.firedSlotIds, ...from.firedSlotIds]);
  const processed = new Set([
    ...into.processedMessageIds,
    ...from.processedMessageIds,
  ]);
  return {
    firedSlotIds: [...fired].slice(-400),
    jobs: [...byId.values()],
    processedMessageIds: [...processed].slice(-200),
    version: 1,
  };
}

export function readBestieJobState(scope: BestieJobScope): BestieJobState {
  return readOwnerScopedState({
    empty: emptyBestieJobState,
    isEmpty: (state) => state.jobs.length === 0 && state.firedSlotIds.length === 0 && state.processedMessageIds.length === 0,
    merge: mergeBestieJobStates,
    parse: parseBestieJobState,
    prefix: BESTIE_JOB_STORAGE_PREFIX,
    scope,
  });
}

export function writeBestieJobState(
  scope: BestieJobScope,
  state: BestieJobState,
): void {
  writeOwnerScopedState(BESTIE_JOB_STORAGE_PREFIX, scope, state);
}

export function createBestieJobId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `bestie-job-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

export function addBestieJob(
  state: BestieJobState,
  input: BestieJobAddInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieJobState {
  const title = input.title.trim();
  const prompt = input.prompt.trim();
  if (!title || !prompt) return state;
  // Dedupe: same title+prompt open/enabled within 10 minutes.
  const needleTitle = title.toLowerCase();
  const needlePrompt = prompt.toLowerCase();
  const dup = state.jobs.find(
    (job) =>
      job.enabled &&
      job.title.toLowerCase() === needleTitle &&
      job.prompt.toLowerCase() === needlePrompt &&
      nowSeconds - job.createdAt <= 600,
  );
  if (dup) return state;

  const enabled = input.enabled !== false;
  let nextDueAt: number | null = null;
  if (enabled) {
    if (input.schedule.kind === "once") {
      nextDueAt =
        input.schedule.dueAt >= nowSeconds ? input.schedule.dueAt : null;
    } else if (input.schedule.kind === "interval") {
      nextDueAt = nowSeconds + Math.max(60, input.schedule.everySeconds);
    } else {
      nextDueAt = computeBestieJobNextDueAt(input.schedule, nowSeconds);
    }
  }
  const job: BestieJob = {
    createdAt: nowSeconds,
    enabled,
    id: createBestieJobId(),
    lastRunAt: null,
    nextDueAt,
    prompt,
    schedule: input.schedule,
    sourceMessageId: input.sourceMessageId ?? null,
    title,
    updatedAt: nowSeconds,
  };
  return { ...state, jobs: [job, ...state.jobs] };
}

export function updateBestieJob(
  state: BestieJobState,
  input: BestieJobUpdateInput,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieJobState {
  return {
    ...state,
    jobs: state.jobs.map((job) => {
      if (job.id !== input.id) return job;
      const schedule = input.schedule ?? job.schedule;
      const enabled = input.enabled ?? job.enabled;
      let nextDueAt = job.nextDueAt;
      if (input.schedule != null || input.enabled != null) {
        if (!enabled) {
          nextDueAt = null;
        } else if (schedule.kind === "once") {
          nextDueAt =
            schedule.dueAt >= nowSeconds ? schedule.dueAt : job.nextDueAt;
        } else {
          nextDueAt = computeBestieJobNextDueAt(schedule, nowSeconds);
        }
      }
      return {
        ...job,
        enabled,
        nextDueAt,
        prompt: input.prompt?.trim() || job.prompt,
        schedule,
        title: input.title?.trim() || job.title,
        updatedAt: nowSeconds,
      };
    }),
  };
}

export function removeBestieJob(
  state: BestieJobState,
  id: string,
): BestieJobState {
  return { ...state, jobs: state.jobs.filter((job) => job.id !== id) };
}

export function markBestieJobMessageProcessed(
  state: BestieJobState,
  messageId: string,
): BestieJobState {
  if (state.processedMessageIds.includes(messageId)) return state;
  const processedMessageIds = [...state.processedMessageIds, messageId];
  const trimmed =
    processedMessageIds.length > 200
      ? processedMessageIds.slice(processedMessageIds.length - 200)
      : processedMessageIds;
  return { ...state, processedMessageIds: trimmed };
}

/**
 * Record a successful fire: mark slot, set lastRunAt, reschedule or disable.
 * Returns null when this slot was already fired (idempotent).
 */
export function markBestieJobFired(
  state: BestieJobState,
  jobId: string,
  dueAt: number,
  nowSeconds = Math.floor(Date.now() / 1000),
): BestieJobState | null {
  const slot = `${jobId}@${dueAt}`;
  if (state.firedSlotIds.includes(slot)) return null;
  const job = state.jobs.find((entry) => entry.id === jobId);
  if (!job) return null;

  let firedSlotIds = [...state.firedSlotIds, slot];
  if (firedSlotIds.length > 400) {
    firedSlotIds = firedSlotIds.slice(firedSlotIds.length - 400);
  }

  const nextDueAt =
    job.schedule.kind === "once"
      ? null
      : computeBestieJobNextDueAt(job.schedule, nowSeconds, { afterRun: true });
  const enabled = job.schedule.kind === "once" ? false : job.enabled;

  return {
    ...state,
    firedSlotIds,
    jobs: state.jobs.map((entry) =>
      entry.id === jobId
        ? {
            ...entry,
            enabled,
            lastRunAt: nowSeconds,
            nextDueAt: enabled ? nextDueAt : null,
            updatedAt: nowSeconds,
          }
        : entry,
    ),
  };
}

export function dueBestieJobs(
  state: BestieJobState,
  nowSeconds: number,
): BestieJob[] {
  return state.jobs.filter((job) => {
    if (!job.enabled) return false;
    if (job.nextDueAt == null) return false;
    if (job.nextDueAt > nowSeconds) return false;
    const slot = `${job.id}@${job.nextDueAt}`;
    return !state.firedSlotIds.includes(slot);
  });
}

export function nextBestieJobDueAt(
  state: BestieJobState,
  nowSeconds: number,
): number | null {
  let next: number | null = null;
  for (const job of state.jobs) {
    if (!job.enabled || job.nextDueAt == null) continue;
    if (job.nextDueAt <= nowSeconds) continue;
    if (next == null || job.nextDueAt < next) next = job.nextDueAt;
  }
  return next;
}

export function enabledJobs(state: BestieJobState): BestieJob[] {
  return state.jobs.filter((job) => job.enabled);
}
