/** Site runbook: agent brief + engineer procedures for a pin or playground sid. */

export type SiteRunbookRef =
  | { kind: "pin"; pinId: string }
  | { kind: "sid"; sid: string };

export type ProcedureStatus = "active" | "pending" | "archived";

export type SiteRunbookProcedure = {
  id: string;
  title: string;
  /** Markdown steps. */
  steps: string;
  status: ProcedureStatus;
  /**
   * Human Persist/lock. When true, agents cannot modify or delete this
   * procedure (MCP propose/upsert is rejected for this title/id).
   */
  persisted?: boolean;
  sourceAgent?: string;
  sourceChannel?: string;
  createdAt: number;
  updatedAt: number;
  acceptedAt?: number;
};

export type SiteRunbook = {
  agentBrief: string;
  procedures: SiteRunbookProcedure[];
  updatedAt: number;
};

/** Compact grant / MCP inject: brief + active procedure index (no full steps). */
export type SiteRunbookInject = {
  agentBrief: string;
  procedures: Array<{
    id: string;
    title: string;
    /** Short plain summary of steps for the index. */
    summary: string;
  }>;
  /** Host-owned Drive protocol; agents must not rewrite. */
  driveProtocol: string[];
};

/** Community pin payload fragment (active procedures only). */
export type SiteRunbookCommunityPayload = {
  agentBrief: string;
  procedures: Array<{
    id: string;
    title: string;
    steps: string;
    createdAt: number;
    updatedAt: number;
    acceptedAt?: number;
    persisted?: boolean;
    sourceAgent?: string;
    sourceChannel?: string;
  }>;
};

export const SITE_RUNBOOK_STORAGE_VERSION = 1 as const;

export type SiteRunbooksBlob = {
  version: typeof SITE_RUNBOOK_STORAGE_VERSION;
  /** Map of encoded key (`pin:…` / `sid:…`) → runbook. */
  runbooks: Record<string, SiteRunbook>;
};
