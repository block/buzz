import type {
  ProcedureStatus,
  SiteRunbook,
  SiteRunbookCommunityPayload,
  SiteRunbookInject,
  SiteRunbookProcedure,
  SiteRunbooksBlob,
} from "./types";
import { SITE_RUNBOOK_STORAGE_VERSION } from "./types";

const STATUSES: ReadonlySet<string> = new Set([
  "active",
  "pending",
  "archived",
]);

export function emptyRunbook(now = Date.now()): SiteRunbook {
  return {
    agentBrief: "",
    procedures: [],
    updatedAt: now,
  };
}

export function emptyRunbooksBlob(): SiteRunbooksBlob {
  return {
    version: SITE_RUNBOOK_STORAGE_VERSION,
    runbooks: {},
  };
}

function asStatus(value: unknown): ProcedureStatus | null {
  if (typeof value !== "string") return null;
  return STATUSES.has(value) ? (value as ProcedureStatus) : null;
}

export function parseProcedure(value: unknown): SiteRunbookProcedure | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  if (typeof candidate.id !== "string" || candidate.id.trim().length === 0) {
    return null;
  }
  const title =
    typeof candidate.title === "string" ? candidate.title.trim() : "";
  if (!title) return null;
  const steps = typeof candidate.steps === "string" ? candidate.steps : "";
  const status = asStatus(candidate.status);
  if (!status) return null;
  const createdAt =
    typeof candidate.createdAt === "number" && Number.isFinite(candidate.createdAt)
      ? candidate.createdAt
      : Date.now();
  const updatedAt =
    typeof candidate.updatedAt === "number" && Number.isFinite(candidate.updatedAt)
      ? candidate.updatedAt
      : createdAt;
  const procedure: SiteRunbookProcedure = {
    id: candidate.id.trim(),
    title,
    steps,
    status,
    createdAt,
    updatedAt,
  };
  if (typeof candidate.sourceAgent === "string" && candidate.sourceAgent.trim()) {
    procedure.sourceAgent = candidate.sourceAgent.trim();
  }
  if (
    typeof candidate.sourceChannel === "string" &&
    candidate.sourceChannel.trim()
  ) {
    procedure.sourceChannel = candidate.sourceChannel.trim();
  }
  if (
    typeof candidate.acceptedAt === "number" &&
    Number.isFinite(candidate.acceptedAt)
  ) {
    procedure.acceptedAt = candidate.acceptedAt;
  }
  return procedure;
}

export function parseSiteRunbook(value: unknown): SiteRunbook | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  const agentBrief =
    typeof candidate.agentBrief === "string" ? candidate.agentBrief : "";
  const procedures: SiteRunbookProcedure[] = [];
  const seen = new Set<string>();
  if (Array.isArray(candidate.procedures)) {
    for (const entry of candidate.procedures) {
      const procedure = parseProcedure(entry);
      if (!procedure || seen.has(procedure.id)) continue;
      seen.add(procedure.id);
      procedures.push(procedure);
    }
  }
  const updatedAt =
    typeof candidate.updatedAt === "number" && Number.isFinite(candidate.updatedAt)
      ? candidate.updatedAt
      : Date.now();
  return { agentBrief, procedures, updatedAt };
}

export function parseSiteRunbooksBlob(value: unknown): SiteRunbooksBlob | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  if (candidate.version !== SITE_RUNBOOK_STORAGE_VERSION) {
    return null;
  }
  const runbooks: Record<string, SiteRunbook> = {};
  if (
    typeof candidate.runbooks === "object" &&
    candidate.runbooks !== null &&
    !Array.isArray(candidate.runbooks)
  ) {
    for (const [key, entry] of Object.entries(
      candidate.runbooks as Record<string, unknown>,
    )) {
      if (!key.trim()) continue;
      const runbook = parseSiteRunbook(entry);
      if (runbook) runbooks[key] = runbook;
    }
  }
  return { version: SITE_RUNBOOK_STORAGE_VERSION, runbooks };
}

/** First non-empty lines of steps, capped for MCP index. */
export function summarizeProcedureSteps(
  steps: string,
  maxLen = 160,
): string {
  const compact = steps
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .join(" ")
    .replace(/\s+/g, " ")
    .trim();
  if (compact.length <= maxLen) return compact;
  return `${compact.slice(0, Math.max(0, maxLen - 1)).trimEnd()}…`;
}

/** Brief + active procedure titles/summaries for grant / poll inject. */
export function shapeRunbookInject(runbook: SiteRunbook): SiteRunbookInject {
  const procedures = runbook.procedures
    .filter((procedure) => procedure.status === "active")
    .map((procedure) => ({
      id: procedure.id,
      title: procedure.title,
      summary: summarizeProcedureSteps(procedure.steps),
    }));
  return {
    agentBrief: runbook.agentBrief.trim(),
    procedures,
  };
}

/** Community pin sync: brief + active procedures (pending stays local). */
export function shapeRunbookForCommunity(
  runbook: SiteRunbook,
): SiteRunbookCommunityPayload | null {
  const agentBrief = runbook.agentBrief.trim();
  const procedures = runbook.procedures
    .filter((procedure) => procedure.status === "active")
    .map((procedure) => {
      const entry: SiteRunbookCommunityPayload["procedures"][number] = {
        id: procedure.id,
        title: procedure.title,
        steps: procedure.steps,
        createdAt: procedure.createdAt,
        updatedAt: procedure.updatedAt,
      };
      if (procedure.acceptedAt != null) entry.acceptedAt = procedure.acceptedAt;
      if (procedure.sourceAgent) entry.sourceAgent = procedure.sourceAgent;
      if (procedure.sourceChannel) entry.sourceChannel = procedure.sourceChannel;
      return entry;
    });
  if (!agentBrief && procedures.length === 0) return null;
  return { agentBrief, procedures };
}

export function parseCommunityRunbookPayload(
  value: unknown,
): SiteRunbook | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  const agentBrief =
    typeof candidate.agentBrief === "string" ? candidate.agentBrief : "";
  const procedures: SiteRunbookProcedure[] = [];
  const seen = new Set<string>();
  if (Array.isArray(candidate.procedures)) {
    for (const entry of candidate.procedures) {
      if (typeof entry !== "object" || entry === null || Array.isArray(entry)) {
        continue;
      }
      const row = entry as Record<string, unknown>;
      const parsed = parseProcedure({
        ...row,
        status: "active",
      });
      if (!parsed || seen.has(parsed.id)) continue;
      seen.add(parsed.id);
      procedures.push(parsed);
    }
  }
  if (!agentBrief.trim() && procedures.length === 0) return null;
  return {
    agentBrief,
    procedures,
    updatedAt: Date.now(),
  };
}

export function createProcedureId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  return `proc-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}
