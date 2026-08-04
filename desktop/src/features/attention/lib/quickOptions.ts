import type { AskType } from "@/features/attention/lib/taskExtraction";

/**
 * The way out (declared-options rule 2): every option set we author
 * ourselves carries an option that declines to answer, so a bad set
 * cannot railroad a wrong one-click answer. Wording is the spec's own
 * example; the click posts it verbatim.
 */
export const MORE_DETAIL_OPTION = "Not enough detail yet, tell me more";

const MAX_OPTION_LENGTH = 60;

/** Auxiliary-verb leads that mark a polar (yes/no) question. */
const YES_NO_LEAD =
  /^(?:do|does|is|are|can|could|should|will|would|has|have|did)\b/i;

/** Leading auxiliary (plus a trailing pronoun) that can be trivially
 * stripped from an A-or-B alternative: "Should we ship Tuesday" → "ship
 * Tuesday". When stripping is not trivially possible the side is used
 * as-is. */
const LEADING_AUXILIARY =
  /^(?:do|does|is|are|can|could|should|will|would|has|have|did)\s+(?:we\s+|i\s+|you\s+|they\s+)?/i;

const OR_WORD = /\bor\b/gi;

function cleanAlternative(text: string): string {
  return text
    .trim()
    .replace(/[,;]+$/, "")
    .trim();
}

function stripLeadingAuxiliary(text: string): string {
  const stripped = text.replace(LEADING_AUXILIARY, "").trim();
  return stripped.length > 0 ? stripped : text;
}

/**
 * "A or B?" asks become their two alternatives — but only with exactly one
 * "or" and two short sides, so mid-sentence prose "or"s never turn into
 * answer buttons.
 */
function eitherOrOptions(ask: string | null): string[] | null {
  if (!ask) {
    return null;
  }
  const trimmed = ask.trim();
  if (!trimmed.endsWith("?")) {
    return null;
  }
  const body = trimmed.replace(/\?+$/, "");
  const orMatches = [...body.matchAll(OR_WORD)];
  if (orMatches.length !== 1) {
    return null;
  }
  const at = orMatches[0].index ?? -1;
  if (at < 0) {
    return null;
  }
  const left = cleanAlternative(body.slice(0, at));
  const right = cleanAlternative(body.slice(at + orMatches[0][0].length));
  if (!left || !right) {
    return null;
  }
  if (left.length > MAX_OPTION_LENGTH || right.length > MAX_OPTION_LENGTH) {
    return null;
  }
  return [stripLeadingAuxiliary(left), right];
}

/**
 * Polar (yes/no) questions: derived questions that end in "?", start with
 * an auxiliary verb, and contain no "or". A which-of-two question ("Does X
 * show A, or is it B?") must never be answerable with Yes/No.
 */
function isPolarQuestion(ask: string): boolean {
  const trimmed = ask.trim();
  return (
    trimmed.endsWith("?") &&
    YES_NO_LEAD.test(trimmed) &&
    !/\bor\b/i.test(trimmed)
  );
}

/**
 * Closed defaults for a declared ask whose author supplied no options
 * (declared-options spec, "Options per response type"). Decision and
 * Question have no safe default — the author must supply the answers,
 * or the free-text path is the way in.
 */
export function defaultDeclaredOptions(type: AskType): string[] {
  switch (type) {
    case "approval":
      return ["Approve", "Reject", MORE_DETAIL_OPTION];
    case "review":
      return ["Looks good", "Changes needed", MORE_DETAIL_OPTION];
    case "blocked":
      return ["I have done it", "I cannot do it", MORE_DETAIL_OPTION];
    default:
      return [];
  }
}

/**
 * Derive one-click reply options for an attention card, per the locked
 * action matrix: A-or-B alternatives > polar yes/no (derived questions
 * only) > the type pairs (Approval and Review). Anything else returns []
 * and the card shows the reply box alone. Blocked is deliberately absent:
 * its primary action is Done, not an option set. Every derived set ends
 * with the way-out option (rule 2 — we are the author here).
 */
export function deriveQuickOptions(
  askType: AskType,
  ask: string | null,
  _content: string,
): string[] {
  const alternatives = eitherOrOptions(ask);
  if (alternatives) {
    return [...alternatives, MORE_DETAIL_OPTION];
  }
  if (askType === "question" && ask && isPolarQuestion(ask)) {
    return ["Yes", "No", MORE_DETAIL_OPTION];
  }
  if (askType === "approval" || askType === "review") {
    return defaultDeclaredOptions(askType);
  }
  return [];
}
