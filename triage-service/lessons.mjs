const MAX_EXAMPLES = 12;
const MAX_PREVIEW_CHARS = 160;
const MAX_NOTE_CHARS = 200;

export const EMPTY_LESSONS = {
  events: new Map(),
  authors: new Map(),
  channels: new Map(),
  threads: new Map(),
  examples: [],
};

function deltaFor(userAction) {
  if (userAction === "dismissed") return -1;
  if (userAction === "done" || userAction === "delegated") return 1;
  return 0;
}

/**
 * Turns the user's triage history into weights and worked examples. A written
 * reason is worth far more than a bare dismissal, so noted rows are picked
 * for the prompt first.
 */
export function buildLessons(feedback) {
  const lessons = {
    events: new Map(),
    authors: new Map(),
    channels: new Map(),
    threads: new Map(),
    examples: [],
  };

  const noted = [];
  const bare = [];

  for (const row of feedback ?? []) {
    const delta = deltaFor(row.userAction);
    if (delta === 0) continue;

    for (const [key, map] of [
      [row.eventId, lessons.events],
      [row.authorPubkey, lessons.authors],
      [row.channelId, lessons.channels],
      [row.threadRootId, lessons.threads],
    ]) {
      if (!key) continue;
      map.set(key, (map.get(key) ?? 0) + delta);
    }

    if (!row.preview) continue;
    const example = {
      preview: String(row.preview).slice(0, MAX_PREVIEW_CHARS),
      userAction: row.userAction,
      note: row.note ? String(row.note).slice(0, MAX_NOTE_CHARS) : null,
    };
    (example.note ? noted : bare).push(example);
  }

  lessons.examples = [...noted, ...bare].slice(0, MAX_EXAMPLES);
  return lessons;
}

/** Prompt fragment shared by both triage stages. */
export function renderCorrections(lessons) {
  const examples = lessons?.examples ?? [];
  if (examples.length === 0) return "";
  const lines = examples.map((example) =>
    example.note
      ? `- "${example.preview}" -> ${example.userAction}, because: ${example.note}`
      : `- "${example.preview}" -> ${example.userAction}`,
  );
  return `\nThe user already judged these. Respect the pattern, and take any stated reason as a standing instruction:\n${lines.join("\n")}\n`;
}
