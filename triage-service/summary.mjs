const SUMMARY_MAX_CHARS = 600;
const SUMMARY_MAX_SENTENCES = 3;

const KIND_LEAD = {
  ask: "asked",
  question: "asked",
  blocker: "flagged a blocker",
  decision: "raised a decision",
  commitment: "committed",
  idea: "shared an idea",
  fyi: "shared",
};

/**
 * Keep a summary as long as it needs to be, but never more than a few
 * sentences. Collapses whitespace so LLM output stays readable in the pane.
 */
export function limitSummary(text) {
  const trimmed = String(text ?? "")
    .replace(/\s+/g, " ")
    .trim();
  if (!trimmed) return "";
  const sentences =
    trimmed.match(/[^.!?]+[.!?]+(?:\s+|$)|[^.!?]+$/g) ?? [trimmed];
  let out = "";
  let count = 0;
  for (const raw of sentences) {
    const sentence = raw.trim();
    if (!sentence) continue;
    const next = out ? `${out} ${sentence}` : sentence;
    if (
      count > 0 &&
      (count >= SUMMARY_MAX_SENTENCES || next.length > SUMMARY_MAX_CHARS)
    ) {
      break;
    }
    out =
      next.length > SUMMARY_MAX_CHARS
        ? `${next.slice(0, SUMMARY_MAX_CHARS - 1).trimEnd()}…`
        : next;
    count += 1;
    if (out.endsWith("…")) break;
  }
  return out;
}

export function headline(content) {
  const line = String(content ?? "").trim().split("\n")[0]?.trim() ?? "";
  if (line.length <= 90) return line;
  return `${line.slice(0, 87).trimEnd()}…`;
}

export function narrativeSummary(kind, message, content) {
  const who = (message.authorLabel || "Someone").trim();
  const where = message.isDm
    ? " in a DM"
    : message.channelName
      ? ` in #${message.channelName}`
      : "";
  const excerpt = limitSummary(content);
  const lead = KIND_LEAD[kind] ?? "wrote";
  if (!excerpt) return limitSummary(`${who} ${lead}${where}.`);
  return limitSummary(`${who} ${lead}${where}: ${excerpt}`);
}
