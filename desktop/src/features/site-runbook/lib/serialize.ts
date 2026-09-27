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

/** Fixed Drive protocol injected with every grant runbook (host-owned). */
export const DRIVE_PROTOCOL: string[] = [
  "Prefer surfaceId (live on every observe/drive response). One browser group only.",
  "browser_snapshot waits for an inline DOM payload — do not stub+poll.",
  "Prefer browser_fill_field for inputs (click+type+verify). Else snapshot then one click/type/key.",
  "browser_drive may set include_snapshot=true to return a fresh snapshot with the action result.",
  "After navigate/click that changes URL: waitFor urlContains or text before next act.",
  "After navigate/load settle, expect a host screen in chat — do not duplicate with screenshot=true unless user asked.",
  "Tabs: browser_tabs lists the group; browser_switch_tab focuses surfaceId (rebinds Drive).",
  "Viewport: browser_get_viewport reads Stage mode/size; browser_set_viewport (Drive) sets desktop|responsive|mobile (responsive W×H; mobile deviceId+orientation+scalePercent).",
  "On no element / no snapshot: waitFor once, re-snapshot once, then stop.",
  "While Taken (userHasControl): follow the human via snapshots + user_click/user_input/user_keydown observe events (valueLength only — never values). Do not Drive until Release.",
  "browser_runbook_propose auto-activates agent procedures. Persisted (human-locked) procedures cannot be changed by agents.",
  "Agent brief is human-owned; do not overwrite it.",
  "Keep goals inside agentBrief + active procedures. Propose new procedures; do not invent sprawl.",
];

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
    typeof candidate.createdAt === "number" &&
    Number.isFinite(candidate.createdAt)
      ? candidate.createdAt
      : Date.now();
  const updatedAt =
    typeof candidate.updatedAt === "number" &&
    Number.isFinite(candidate.updatedAt)
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
  if (
    typeof candidate.sourceAgent === "string" &&
    candidate.sourceAgent.trim()
  ) {
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
  if (candidate.persisted === true) {
    procedure.persisted = true;
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
    typeof candidate.updatedAt === "number" &&
    Number.isFinite(candidate.updatedAt)
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
export function summarizeProcedureSteps(steps: string, maxLen = 160): string {
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
    driveProtocol: [...DRIVE_PROTOCOL],
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
      if (procedure.persisted) entry.persisted = true;
      if (procedure.sourceAgent) entry.sourceAgent = procedure.sourceAgent;
      if (procedure.sourceChannel)
        entry.sourceChannel = procedure.sourceChannel;
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
