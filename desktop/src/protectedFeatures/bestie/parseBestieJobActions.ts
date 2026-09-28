import type { BestieJobAddInput, BestieJobSchedule } from "./bestieJobTypes";

/**
 * Structured Bestie job mutations in agent chat:
 *
 * ```bestie-job
 * {"op":"add","confirmed":true,"job":{"title":"Morning brief","prompt":"Summarize calendar","schedule":{"kind":"daily","hour":9,"minute":0}}}
 * ```
 *
 * Chat creates only via `add` with `confirmed: true` (after user approval).
 * `draft` / unconfirmed `add` parse for protocol completeness but do not create.
 */
export type BestieJobAction =
  | { confirmed: true; job: BestieJobAddInput; op: "add" }
  | { job: BestieJobAddInput; op: "draft" }
  | {
      enabled?: boolean;
      id: string;
      op: "update";
      prompt?: string;
      schedule?: BestieJobSchedule;
      title?: string;
    }
  | { id: string; op: "remove" };

const FENCE_RE = /```bestie-job\s*\r?\n([\s\S]*?)```/gi;

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

function parseAddJob(value: unknown): BestieJobAddInput | null {
  if (typeof value !== "object" || value === null) return null;
  const record = value as Record<string, unknown>;
  if (typeof record.title !== "string" || typeof record.prompt !== "string") {
    return null;
  }
  const title = record.title.trim();
  const prompt = record.prompt.trim();
  const schedule = parseSchedule(record.schedule);
  if (!title || !prompt || !schedule) return null;
  return {
    enabled: record.enabled !== false,
    prompt,
    schedule,
    title,
  };
}

export function parseBestieJobActionPayload(value: unknown): BestieJobAction[] {
  const payloads = Array.isArray(value) ? value : [value];
  const actions: BestieJobAction[] = [];
  for (const payload of payloads) {
    if (typeof payload !== "object" || payload === null) continue;
    const record = payload as Record<string, unknown>;
    if (record.op === "add" || record.op === "draft") {
      const job = parseAddJob(record.job);
      if (!job) continue;
      // Create only when add + confirmed:true. Unconfirmed add → draft (no create).
      if (record.op === "add" && record.confirmed === true) {
        actions.push({ confirmed: true, job, op: "add" });
      } else {
        actions.push({ job, op: "draft" });
      }
      continue;
    }
    if (
      record.op === "remove" &&
      typeof record.id === "string" &&
      record.id.length > 0
    ) {
      actions.push({ id: record.id, op: "remove" });
      continue;
    }
    if (
      record.op === "update" &&
      typeof record.id === "string" &&
      record.id.length > 0
    ) {
      const schedule =
        record.schedule === undefined
          ? undefined
          : (parseSchedule(record.schedule) ?? undefined);
      if (record.schedule !== undefined && schedule === undefined) continue;
      actions.push({
        enabled:
          typeof record.enabled === "boolean" ? record.enabled : undefined,
        id: record.id,
        op: "update",
        prompt: typeof record.prompt === "string" ? record.prompt : undefined,
        schedule,
        title: typeof record.title === "string" ? record.title : undefined,
      });
    }
  }
  return actions;
}

function tryParseJson(raw: string): unknown | null {
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

export function parseBestieJobActionsFromMessage(
  content: string,
): BestieJobAction[] {
  const actions: BestieJobAction[] = [];
  for (const match of content.matchAll(FENCE_RE)) {
    const json = tryParseJson(match[1]?.trim() ?? "");
    if (json != null) actions.push(...parseBestieJobActionPayload(json));
  }
  if (actions.length > 0) return actions;

  const trimmed = content.trim();
  if (
    (trimmed.startsWith("{") || trimmed.startsWith("[")) &&
    trimmed.includes('"op"') &&
    (trimmed.includes('"job"') || trimmed.includes("bestie-job"))
  ) {
    const json = tryParseJson(trimmed);
    if (json != null) return parseBestieJobActionPayload(json);
  }
  return [];
}
