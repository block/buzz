/**
 * Documented Bestie list protocol — teach the agent (and UI) how to mutate
 * reminders / to-dos via a fenced `bestie-list` JSON block.
 *
 * Desktop also applies a client-side natural-language path so “remind me…”
 * works even when the agent forgets the fence — except bare clocks without
 * AM/PM, which need confirm (AM vs PM or next-occurrence) before create.
 */

export const BESTIE_LIST_FENCE_LANG = "bestie-list";

/** Marker used to append / strip turn hints from outbound Bestie messages. */
export const BESTIE_LIST_TURN_HINT_MARKER = "[Bestie lists]";

/**
 * Standing instructions for the Bestie agent. Keep stable — spawn/snapshot
 * injectors and outbound turn hints share this text.
 */
export const BESTIE_LIST_AGENT_INSTRUCTIONS = `You are the user's Bestie. When they ask you to add, complete, or remove a reminder or to-do, you MUST include a fenced ${BESTIE_LIST_FENCE_LANG} JSON block in your reply so the desktop list updates. Do not only describe the change in prose.

Fence format (one JSON object or an array of objects):

\`\`\`${BESTIE_LIST_FENCE_LANG}
{"op":"add","items":[{"kind":"reminder","text":"Call mom","dueAt":1735689600000}]}
\`\`\`

Ops:
- add: items[] with kind "todo" | "reminder", text (required), dueAt unix **milliseconds** (optional; reminders only; Date.now()-style absolute. Seconds-scale values are coerced on apply)
- complete: {"op":"complete","id":"<item-id>"}
- remove: {"op":"remove","id":"<item-id>"}

Reminder text must be **crystallized**: clean imperative, capitalize the first letter, drop a leading "to"/"for". Example: user said "remind me to run the nightly report" → fence text "Run the nightly report" (due time only in dueAt, never in text).

Bare clock without AM/PM (e.g. "at 8:36"): do **not** invent am/pm and do **not** create yet. Ask AM vs PM (or propose the next occurrence from now) **in your reply in this same thread** — parent to the user ask / session root; never post a new top-level Assistant DM for the confirm question. Wait for explicit confirm. The desktop resolves dueAt from the user's AM/PM reply; if you still fence, dueAt MUST match the confirmed meridiem (evening "PM" must not land as next-morning AM). Relative times ("in 20 minutes") and clocks with am/pm (or 24h hours 13–23) may be fenced immediately.

If the user did not give a due time for a reminder, omit dueAt (or set null). Confirm briefly in natural language in addition to the fence.

The desktop may already apply the user's natural-language add before your reply when the time is unambiguous. Prefer acknowledging without a second add when the list already shows the item. If you still emit add, put the crystallized task text only (due time in dueAt, not in text); the client dedupes by core text + due window and prefers the row with dueAt.

For scheduled *jobs* (auto-run a prompt later), follow the Bestie job confirmation protocol (clarify → exact plan → user approve → fenced bestie-job add with confirmed:true). Do not treat jobs like instant reminders.

Each user turn includes a [Bestie live lists] block with the current owner-scoped RHS snapshot (open to-dos, open reminders, jobs summary). When the user asks what is on their to-do list, reminders, or jobs, answer from that live block. Do not claim you cannot query live state; do not prefer session chat memory over the live snapshot.`;

export type BestieListTurnHintOptions = {
  /** Strengthen hint when this turn has a bare clock needing AM/PM confirm. */
  bareClockConfirm?: boolean;
};

/** Compact turn hint appended to Bestie user messages that look like list intents. */
export function bestieListTurnHint(
  options: BestieListTurnHintOptions = {},
): string {
  const bare = options.bareClockConfirm
    ? `
Bare clock without AM/PM in this message: do NOT create a reminder yet. Ask AM vs PM (or propose the next occurrence) in your reply in this same thread (never a new top-level Assistant DM). Wait for explicit confirm. Prefer letting the desktop apply dueAt from the user's AM/PM reply; if you fence, dueAt must match confirmed meridiem + crystallized text (e.g. "Run the nightly report").`
    : `
Reminder text: crystallize as a clean imperative (capitalize; drop leading "to"). Bare clock like "8:36" with no AM/PM: ask AM vs PM in-thread (not a new top-level DM) or propose next occurrence before fencing — do not invent am/pm; fence dueAt must match confirmed AM/PM.`;

  return `

${BESTIE_LIST_TURN_HINT_MARKER}
When mutating reminders/todos, emit a fenced ${BESTIE_LIST_FENCE_LANG} JSON block, e.g.
\`\`\`${BESTIE_LIST_FENCE_LANG}
{"op":"add","items":[{"kind":"reminder","text":"Run the nightly report","dueAt":1735689600000}]}
\`\`\`
(dueAt = unix milliseconds; seconds also accepted and coerced. Omit if unknown). Also support complete/remove by id.${bare}`;
}

/** Strip the outbound turn hint (and anything after the marker) for UI display. */
export function stripBestieListTurnHint(
  content: string | null | undefined,
): string {
  if (typeof content !== "string") return "";
  const index = content.indexOf(`\n\n${BESTIE_LIST_TURN_HINT_MARKER}`);
  if (index >= 0) return content.slice(0, index).trimEnd();
  const alt = content.indexOf(BESTIE_LIST_TURN_HINT_MARKER);
  if (alt >= 0 && (alt === 0 || content[alt - 1] === "\n")) {
    return content.slice(0, alt).trimEnd();
  }
  return content;
}

/** Append the turn hint when the body looks like a list mutation request. */
export function withBestieListTurnHint(
  content: string,
  shouldAttach: boolean,
  options: BestieListTurnHintOptions = {},
): string {
  if (!shouldAttach) return content;
  if (content.includes(BESTIE_LIST_TURN_HINT_MARKER)) return content;
  return `${content.trimEnd()}${bestieListTurnHint(options)}`;
}
