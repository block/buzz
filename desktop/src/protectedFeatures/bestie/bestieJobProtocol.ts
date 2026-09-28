/**
 * Bestie Jobs protocol — agent fence + turn hints for scheduled prompts.
 *
 * Chat path requires clarify → exact plan → explicit user approve → confirmed
 * fence. RHS / manual form-complete add bypasses this and creates directly.
 */

export const BESTIE_JOB_FENCE_LANG = "bestie-job";

export const BESTIE_JOB_TURN_HINT_MARKER = "[Bestie jobs]";

export const BESTIE_JOB_AGENT_INSTRUCTIONS = `When the user asks to schedule a recurring or one-time *job* (auto-run a prompt later — not a simple reminder nudge), do **not** create it immediately. Jobs are not like reminders: chat setup needs a confirmation protocol.

Required chat flow:
1. Ask clarifying questions until you know: schedule type (one-shot | interval | daily | weekly) and the fields that type needs (when / interval length / time of day / weekday), plus the prompt / what to run.
2. Before creating, present an **exact approval plan** in plain language covering:
   - What will run (full prompt text; if messaging/mentioning someone, say so)
   - When (concrete schedule: one-shot time, interval, daily time, or weekly weekday+time)
   - Concrete targets when relevant (channel id, thread id, recipients, @mentions, etc.)
3. Wait for the user to explicitly approve that plan (e.g. "yes", "approve", "create it", "go ahead").
4. Only then emit a fenced ${BESTIE_JOB_FENCE_LANG} JSON block with "confirmed": true.

\`\`\`${BESTIE_JOB_FENCE_LANG}
{"op":"add","confirmed":true,"job":{"title":"Morning brief","prompt":"Summarize my calendar and inbox.","schedule":{"kind":"daily","hour":9,"minute":0}}}
\`\`\`

Optional while clarifying (does **not** create): {"op":"draft","job":{…}} — same job shape, no confirmed flag.

Schedule kinds:
- once: {"kind":"once","dueAt":<unix seconds>}
- interval: {"kind":"interval","everySeconds":3600}
- daily: {"kind":"daily","hour":9,"minute":0}
- weekly: {"kind":"weekly","weekday":1,"hour":9,"minute":0} (0=Sunday)

Also support {"op":"update","id":"…","enabled":false} and {"op":"remove","id":"…"}.
Never emit add with confirmed:true until the user approves the exact plan. The desktop ignores unconfirmed add / draft fences (RHS manual add can still create form-complete jobs directly).
Jobs differ from reminders: when due, the desktop wakes you and sends the job prompt as a user turn for you to execute.`;

export function bestieJobTurnHint(): string {
  return `

${BESTIE_JOB_TURN_HINT_MARKER}
Job setup (chat): clarify schedule type (once|interval|daily|weekly) + needed fields, then present an exact plan (prompt text, when, concrete targets like channel/thread/recipients). Wait for explicit user approve. Only then emit a fenced ${BESTIE_JOB_FENCE_LANG} add with "confirmed":true. Do not auto-create from a casual "schedule a job…" — draft/unconfirmed fences do not create.`;
}

export function stripBestieJobTurnHint(
  content: string | null | undefined,
): string {
  if (typeof content !== "string") return "";
  const index = content.indexOf(`\n\n${BESTIE_JOB_TURN_HINT_MARKER}`);
  if (index >= 0) return content.slice(0, index).trimEnd();
  const alt = content.indexOf(BESTIE_JOB_TURN_HINT_MARKER);
  if (alt >= 0 && (alt === 0 || content[alt - 1] === "\n")) {
    return content.slice(0, alt).trimEnd();
  }
  return content;
}

export function withBestieJobTurnHint(
  content: string,
  shouldAttach: boolean,
): string {
  if (!shouldAttach) return content;
  if (content.includes(BESTIE_JOB_TURN_HINT_MARKER)) return content;
  return `${content.trimEnd()}${bestieJobTurnHint()}`;
}
