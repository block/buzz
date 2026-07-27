export type CrmActionCard = {
  actionId: string;
  actionType: "reddit_mark_posted" | "lead_categorize" | "outreach_approve";
  expiresAt: string;
  content: string;
};

const MARKER = /(?:^|\n)crm-action:v1:([0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}):(reddit_mark_posted|lead_categorize|outreach_approve):(\S+)\s*$/i;
const REDDIT_DRAFT = /(?:^|\n)Draft to copy manually:\s*\n(`{3,})[^\n]*\n([\s\S]*?)\n\1(?=\n|$)/i;
const ACTION_HEADER = /^\[CRM Buzz action [0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}\]\s*\n?/i;
const ACTION_FOOTERS: Record<CrmActionCard["actionType"], RegExp> = {
  reddit_mark_posted: /\n*Mark Reddit draft as posted\.\nUse Approve only after the Reddit post is live\.\nExpires: [^\n]+\s*$/i,
  lead_categorize: /\n*Choose a lead category using the action card\.\nThe selected category will be recorded and queued for SmartLead synchronization\.\nExpires: [^\n]+\s*$/i,
  outreach_approve: /\n*Approve to send this frozen draft, or reject it\.\nExpires: [^\n]+\s*$/i,
};

function readableContent(body: string, actionType: CrmActionCard["actionType"]): string {
  return body.replace(ACTION_HEADER, "").replace(ACTION_FOOTERS[actionType], "").trim();
}

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
    content: readableContent(body.slice(0, match.index), match[2] as CrmActionCard["actionType"]),
  };
}

/** Return only the explicitly delimited read-only Reddit draft, if present. */
export function extractCrmRedditDraft(content: string): string | null {
  const match = REDDIT_DRAFT.exec(content);
  const draft = match?.[2]?.trim();
  return draft || null;
}
