/** Portable browser share: URL + site runbook (no cookies / auth). */

import type {
  ProcedureStatus,
  SiteRunbookProcedure,
} from "@/features/site-runbook/lib/types";

export const HULA_BROWSER_SHARE_KIND = "hula-browser-share.v1" as const;

export type BrowserShareSource = "pin" | "session";

/** Procedures included in a share (active + pending; archived omitted). */
export type BrowserShareProcedure = Pick<
  SiteRunbookProcedure,
  | "id"
  | "title"
  | "steps"
  | "status"
  | "createdAt"
  | "updatedAt"
  | "acceptedAt"
  | "sourceAgent"
  | "sourceChannel"
> & {
  status: Extract<ProcedureStatus, "active" | "pending">;
};

export type BrowserShareRunbook = {
  agentBrief: string;
  procedures: BrowserShareProcedure[];
};

/**
 * Versioned JSON for Export / Import.
 * Never includes cookies, passwords, or login sessions.
 */
export type HulaBrowserShareV1 = {
  kind: typeof HULA_BROWSER_SHARE_KIND;
  url: string;
  title?: string;
  exportedAt: number;
  source: BrowserShareSource;
  runbook: BrowserShareRunbook;
};

export const BROWSER_SHARE_SECURITY_NOTE =
  "This file has URL and site runbook only. It does not include cookies, passwords, or login sessions.";
