export const MEMORY_PROPOSAL_PREFIX = "mem/proposals/";

export const MEMORY_PROPOSAL_KINDS = [
  "fact",
  "preference",
  "policy",
  "procedure",
  "delegation-role",
] as const;
export type MemoryProposalKind = (typeof MEMORY_PROPOSAL_KINDS)[number];

export const MEMORY_PROPOSAL_SCOPES = ["agent", "owner"] as const;
export type MemoryProposalScope = (typeof MEMORY_PROPOSAL_SCOPES)[number];

export type MemoryProposal = {
  schema: 1;
  status: "proposed" | "approved" | "rejected" | "undone";
  kind: MemoryProposalKind;
  scope: MemoryProposalScope;
  targetSlug: string;
  content: string;
  reason: string;
  sourceEventIds: string[];
  evidenceIds: string[];
  confidence?: number;
  previousValue?: string | null;
  reviewedAt?: number;
  targetEventId?: string;
};

const MEMORY_SLUG =
  /^mem\/[a-z0-9][a-z0-9_-]{0,63}(\/[a-z0-9][a-z0-9_-]{0,63})*$/;
const EVENT_ID = /^[0-9a-f]{64}$/i;

export function parseMemoryProposal(
  slug: string,
  body: string,
): MemoryProposal | null {
  if (!slug.startsWith(MEMORY_PROPOSAL_PREFIX)) return null;
  let value: unknown;
  try {
    value = JSON.parse(body);
  } catch {
    return null;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const candidate = value as Partial<MemoryProposal>;
  if (
    candidate.schema !== 1 ||
    !["proposed", "approved", "rejected", "undone"].includes(
      candidate.status ?? "",
    ) ||
    !MEMORY_PROPOSAL_KINDS.includes(candidate.kind as MemoryProposalKind) ||
    !MEMORY_PROPOSAL_SCOPES.includes(candidate.scope as MemoryProposalScope) ||
    typeof candidate.targetSlug !== "string" ||
    !MEMORY_SLUG.test(candidate.targetSlug) ||
    candidate.targetSlug.startsWith(MEMORY_PROPOSAL_PREFIX) ||
    typeof candidate.content !== "string" ||
    candidate.content.trim().length === 0 ||
    typeof candidate.reason !== "string" ||
    candidate.reason.trim().length === 0 ||
    !isEventIds(candidate.sourceEventIds) ||
    !isEventIds(candidate.evidenceIds)
  )
    return null;
  if (
    candidate.confidence !== undefined &&
    (typeof candidate.confidence !== "number" ||
      candidate.confidence < 0 ||
      candidate.confidence > 1)
  )
    return null;
  return candidate as MemoryProposal;
}

function isEventIds(value: unknown): value is string[] {
  return (
    Array.isArray(value) &&
    value.every((id) => typeof id === "string" && EVENT_ID.test(id))
  );
}
