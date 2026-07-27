export type CrmActionCard = {
  actionId: string;
  actionType:
    | "reddit_mark_posted"
    | "lead_categorize"
    | "outreach_approve"
    | "calendar_book"
    | "lead_control";
  expiresAt: string;
  content: string;
  calendarSlots?: CrmCalendarSlot[];
  leadControlChoices?: string[];
};

export type CrmCalendarSlot = { label: string; reaction: string };

const CONTROL_REACTIONS: Record<CrmActionCard["actionType"], readonly string[]> = {
  reddit_mark_posted: ["✅", "❌"],
  lead_categorize: ["👍", "📅", "ℹ️", "👎", "🕒", "⛔", "🔀", "❌"],
  outreach_approve: ["✅", "❌", "✏️"],
  calendar_book: ["1️⃣", "2️⃣", "3️⃣", "❌"],
  lead_control: ["⛔", "🏢", "🗑️", "✅"],
};

const MARKER =
  /(?:^|\n)crm-action:v1:([0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}):(reddit_mark_posted|lead_categorize|outreach_approve|calendar_book|lead_control):(\S+)\s*$/i;
const REDDIT_DRAFT =
  /(?:^|\n)Draft to copy manually:\s*\n(`{3,})[^\n]*\n([\s\S]*?)\n\1(?=\n|$)/i;
const CALENDAR_SLOT = /^\*\*Slot ([1-3]):\*\*\s*(.+)$/gim;
const LEAD_CONTROL_OPTIONS =
  /(?:^|\n)crm-action-options:v1:lead_control:([^\n]+)\s*(?=\n|$)/i;
const LEAD_CONTROL_REACTIONS = new Set(["⛔", "🏢", "🗑️"]);
const ACTION_HEADER =
  /^\[CRM Buzz action [0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}\]\s*\n?/i;
const ACTION_FOOTERS: Record<CrmActionCard["actionType"], RegExp> = {
  reddit_mark_posted:
    /\n*Mark Reddit draft as posted\.\nUse Approve only after the Reddit post is live\.\nExpires: [^\n]+\s*$/i,
  lead_categorize:
    /\n*Choose a lead category using the action card\.\nThe selected category will be recorded and queued for SmartLead synchronization\.\nExpires: [^\n]+\s*$/i,
  outreach_approve:
    /\n*Approve to send this frozen draft, or reject it\.\nExpires: [^\n]+\s*$/i,
  calendar_book:
    /\n*Choose a meeting slot using the action card\.\nExpires: [^\n]+\s*$/i,
  lead_control:
    /\n*Choose a safeguard using the action card\.\nExpires: [^\n]+\s*$/i,
};

function readableContent(
  body: string,
  actionType: CrmActionCard["actionType"],
): string {
  const readable = body
    .replace(ACTION_HEADER, "")
    .replace(ACTION_FOOTERS[actionType], "")
    .replace(LEAD_CONTROL_OPTIONS, "");

  return (actionType === "calendar_book"
    ? readable.replace(CALENDAR_SLOT, "").replace(/\n{3,}/g, "\n\n")
    : readable
  ).trim();
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

  const action: CrmActionCard = {
    actionId: match[1],
    actionType: match[2] as CrmActionCard["actionType"],
    expiresAt,
    content: readableContent(
      body.slice(0, match.index),
      match[2] as CrmActionCard["actionType"],
    ),
  };

  if (action.actionType === "calendar_book") {
    action.calendarSlots = extractCrmCalendarSlots(body);
  }
  if (action.actionType === "lead_control") {
    const choices = extractCrmLeadControlChoices(body);
    if (choices !== undefined) {
      action.leadControlChoices = choices;
    }
  }

  return action;
}

/** Action reactions are signed transport controls, not conversation feedback. */
export function isCrmActionControlReaction(
  action: CrmActionCard | null,
  emoji: string,
): boolean {
  return Boolean(action && CONTROL_REACTIONS[action.actionType].includes(emoji));
}

/** Return only the explicitly delimited read-only Reddit draft, if present. */
export function extractCrmRedditDraft(content: string): string | null {
  const match = REDDIT_DRAFT.exec(content);
  const draft = match?.[2]?.trim();
  return draft || null;
}

/** Return the booked-slot choices that CRM explicitly rendered for this card. */
export function extractCrmCalendarSlots(
  content: string,
): CrmCalendarSlot[] {
  const slots: CrmCalendarSlot[] = [];
  const seen = new Set<number>();

  for (const match of content.matchAll(CALENDAR_SLOT)) {
    const slotNumber = Number(match[1]);
    const label = match[2]?.trim();
    if (!label || seen.has(slotNumber)) continue;
    seen.add(slotNumber);
    slots.push({ label, reaction: `${slotNumber}\uFE0F\u20E3` });
  }

  return slots;
}

/** Return only the safeguard choices frozen by CRM for this review. */
export function extractCrmLeadControlChoices(
  content: string,
): string[] | undefined {
  const encoded = LEAD_CONTROL_OPTIONS.exec(content)?.[1];
  if (!encoded) return undefined;
  return [...new Set(encoded.split(",").map((emoji) => emoji.trim()))].filter(
    (emoji) => LEAD_CONTROL_REACTIONS.has(emoji),
  );
}
