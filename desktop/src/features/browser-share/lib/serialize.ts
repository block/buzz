import { normalizeBrowserSessionUrl } from "@/features/browsers/lib/addBrowserSession";
import type { SiteRunbook } from "@/features/site-runbook/lib/types";
import { parseProcedure } from "@/features/site-runbook/lib/serialize";

import {
  BROWSER_SHARE_SECURITY_NOTE,
  HULA_BROWSER_SHARE_KIND,
  type BrowserShareProcedure,
  type BrowserShareRunbook,
  type BrowserShareSource,
  type HulaBrowserShareV1,
} from "./types";

export { BROWSER_SHARE_SECURITY_NOTE, HULA_BROWSER_SHARE_KIND };

function shapeShareProcedure(
  procedure: SiteRunbook["procedures"][number],
): BrowserShareProcedure | null {
  if (procedure.status !== "active" && procedure.status !== "pending") {
    return null;
  }
  const entry: BrowserShareProcedure = {
    id: procedure.id,
    title: procedure.title,
    steps: procedure.steps,
    status: procedure.status,
    createdAt: procedure.createdAt,
    updatedAt: procedure.updatedAt,
  };
  if (procedure.acceptedAt != null) entry.acceptedAt = procedure.acceptedAt;
  if (procedure.sourceAgent) entry.sourceAgent = procedure.sourceAgent;
  if (procedure.sourceChannel) entry.sourceChannel = procedure.sourceChannel;
  return entry;
}

/** Build export payload from URL + local runbook. Archived procedures omitted. */
export function buildBrowserShare(input: {
  url: string;
  title?: string;
  source: BrowserShareSource;
  runbook: SiteRunbook;
  exportedAt?: number;
}): HulaBrowserShareV1 | null {
  const url = normalizeBrowserSessionUrl(input.url);
  if (!url) return null;
  const title = input.title?.trim() || undefined;
  const procedures: BrowserShareProcedure[] = [];
  const seen = new Set<string>();
  for (const procedure of input.runbook.procedures) {
    const shaped = shapeShareProcedure(procedure);
    if (!shaped || seen.has(shaped.id)) continue;
    seen.add(shaped.id);
    procedures.push(shaped);
  }
  const share: HulaBrowserShareV1 = {
    kind: HULA_BROWSER_SHARE_KIND,
    url,
    exportedAt: input.exportedAt ?? Date.now(),
    source: input.source,
    runbook: {
      agentBrief: input.runbook.agentBrief ?? "",
      procedures,
    },
  };
  if (title) share.title = title;
  return share;
}

export function browserShareToJson(share: HulaBrowserShareV1): string {
  return `${JSON.stringify(share, null, 2)}\n`;
}

function parseShareProcedure(value: unknown): BrowserShareProcedure | null {
  const parsed = parseProcedure(value);
  if (!parsed) return null;
  if (parsed.status !== "active" && parsed.status !== "pending") return null;
  return {
    id: parsed.id,
    title: parsed.title,
    steps: parsed.steps,
    status: parsed.status,
    createdAt: parsed.createdAt,
    updatedAt: parsed.updatedAt,
    ...(parsed.acceptedAt != null ? { acceptedAt: parsed.acceptedAt } : {}),
    ...(parsed.sourceAgent ? { sourceAgent: parsed.sourceAgent } : {}),
    ...(parsed.sourceChannel ? { sourceChannel: parsed.sourceChannel } : {}),
  };
}

function parseShareRunbook(value: unknown): BrowserShareRunbook | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  const agentBrief =
    typeof candidate.agentBrief === "string" ? candidate.agentBrief : "";
  const procedures: BrowserShareProcedure[] = [];
  const seen = new Set<string>();
  if (Array.isArray(candidate.procedures)) {
    for (const entry of candidate.procedures) {
      const procedure = parseShareProcedure(entry);
      if (!procedure || seen.has(procedure.id)) continue;
      seen.add(procedure.id);
      procedures.push(procedure);
    }
  }
  return { agentBrief, procedures };
}

/** Parse and validate a share JSON value. Wrong kind / bad URL → null. */
export function parseBrowserShare(value: unknown): HulaBrowserShareV1 | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  if (candidate.kind !== HULA_BROWSER_SHARE_KIND) return null;
  if (typeof candidate.url !== "string") return null;
  const url = normalizeBrowserSessionUrl(candidate.url);
  if (!url) return null;
  if (candidate.source !== "pin" && candidate.source !== "session") return null;
  const runbook = parseShareRunbook(candidate.runbook);
  if (!runbook) return null;
  const exportedAt =
    typeof candidate.exportedAt === "number" &&
    Number.isFinite(candidate.exportedAt)
      ? candidate.exportedAt
      : Date.now();
  const share: HulaBrowserShareV1 = {
    kind: HULA_BROWSER_SHARE_KIND,
    url,
    exportedAt,
    source: candidate.source,
    runbook,
  };
  if (typeof candidate.title === "string" && candidate.title.trim()) {
    share.title = candidate.title.trim();
  }
  return share;
}

export function parseBrowserShareJson(raw: string): HulaBrowserShareV1 | null {
  try {
    return parseBrowserShare(JSON.parse(raw));
  } catch {
    return null;
  }
}

/** Map share runbook into the local store shape (preserves pending). */
export function shareRunbookToSiteRunbook(
  runbook: BrowserShareRunbook,
  now = Date.now(),
): SiteRunbook {
  return {
    agentBrief: runbook.agentBrief,
    procedures: runbook.procedures.map((procedure) => ({
      ...procedure,
    })),
    updatedAt: now,
  };
}

export function suggestBrowserShareFilename(share: HulaBrowserShareV1): string {
  let host = "browser";
  try {
    host = new URL(share.url).hostname || host;
  } catch {
    // keep default
  }
  const safe = host.replace(/[^a-zA-Z0-9._-]+/g, "-").slice(0, 48) || "browser";
  return `${safe}.hula-browser-share.json`;
}
