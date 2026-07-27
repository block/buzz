export type CrmActionCard = {
  actionId: string;
  actionType: "reddit_mark_posted" | "lead_categorize" | "outreach_approve";
  expiresAt: string;
  content: string;
};

const MARKER = /(?:^|\n)crm-action:v1:([0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}):(reddit_mark_posted|lead_categorize|outreach_approve):(\S+)\s*$/i;
const REDDIT_DRAFT = /(?:^|\n)Draft to copy manually:\s*\n```[^\n]*\n([\s\S]*?)\n```/i;

/**
 * Parses CRM's versioned action marker. The marker is removed before Markdown
 * rendering, so it stays a transport contract rather than visible UI chrome.
 */
export function parseCrmActionCard(body: string): CrmActionCard | null {
  const match = MARKER.exec(body);
  if (!match) return null;

  const expiresAt = match[3];
  if (!Number.isFinite(Date.parse(expiresAt))) return null;

  return {
    actionId: match[1],
    actionType: match[2] as CrmActionCard["actionType"],
    expiresAt,
    content: body.slice(0, match.index).trim(),
  };
}

/** Return only the explicitly delimited read-only Reddit draft, if present. */
export function extractCrmRedditDraft(content: string): string | null {
  const match = REDDIT_DRAFT.exec(content);
  const draft = match?.[1]?.trim();
  return draft || null;
}
