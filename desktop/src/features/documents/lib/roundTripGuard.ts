/**
 * The gate that decides whether a note is safe to edit in live preview.
 *
 * A WYSIWYG editor autosaving over a real Obsidian vault is a data-destruction
 * feature unless proven otherwise: tiptap-markdown is markdown-it plus
 * prosemirror-markdown, and anything outside the TipTap schema does not
 * survive the trip. Measured against a real vault it will, among other things,
 * turn `[[link]]` into `\[\[link\]\]`, normalize bullet markers and list
 * indentation, rewrite `_em_` as `*em*`, and drop callouts and footnotes.
 *
 * Rather than guess which constructs are safe, we simply ask: does
 * `serialize(parse(x))` return `x`? If not, the file opens in source mode --
 * a raw textarea that never touches the serializer -- and the user is told
 * why. Editing is still possible; silent reformatting is not.
 */

export type RoundTripStatus = "stable" | "lossy" | "unknown";

/** Lines that begin a block and must never be joined to the previous one. */
const BLOCK_START =
  /^(?:\s{4,}|\t)|^ {0,3}(?:#{1,6}\s|>|[-*+]\s|\d+[.)]\s|\||`{3,}|~{3,}|---|===|\[\^)/;

/**
 * Joins soft-wrapped paragraph lines, the way a CommonMark serializer does.
 *
 * Hard-wrapping prose at ~80 columns is extremely common, and a single newline
 * inside a paragraph is a *soft* break: it carries no meaning, and the
 * serializer legitimately re-emits the paragraph as one line. Comparing raw
 * bytes therefore flagged almost every real-world note as lossy — measured at
 * 35 of 40 files in this repo — which pushed everything into source mode and
 * made live preview pointless.
 *
 * The join is deliberately conservative. Any line that could begin a block
 * (list item, heading, quote, table row, fence, indented code) stops the join,
 * and fenced regions are skipped entirely. Failing to join something merely
 * routes that file to source mode, which is the safe direction.
 */
function joinSoftWrappedLines(text: string): string {
  const lines = text.split("\n");
  const joined: string[] = [];
  let inFence = false;

  for (const line of lines) {
    if (/^\s*(?:`{3,}|~{3,})/.test(line)) {
      inFence = !inFence;
      joined.push(line);
      continue;
    }

    const previous = joined.at(-1);
    const canJoin =
      !inFence &&
      previous !== undefined &&
      previous.trim() !== "" &&
      line.trim() !== "" &&
      !BLOCK_START.test(line) &&
      !BLOCK_START.test(previous) &&
      // Two trailing spaces are an explicit hard break; preserve it.
      !/ {2}$/.test(previous);

    if (canJoin) {
      joined[joined.length - 1] = `${previous} ${line.trim()}`;
    } else {
      joined.push(line);
    }
  }

  return joined.join("\n");
}

/**
 * A GFM table delimiter row, e.g. `| :--- | ---: |`.
 *
 * The serializer emits a canonical three-dash form, so a hand-aligned source
 * row differs only in dash count — which carries no meaning. Alignment colons
 * do, and are preserved.
 */
const TABLE_DELIMITER_ROW = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

function normalizeTableDelimiters(text: string): string {
  return text
    .split("\n")
    .map((line) =>
      TABLE_DELIMITER_ROW.test(line)
        ? line.replace(/-{2,}/g, "---").replace(/\s+/g, " ").trim()
        : line,
    )
    .join("\n");
}

/**
 * Differences that are cosmetic rather than corrupting, and are normalized away
 * before comparison:
 *
 *  - CRLF vs LF. Writing back LF is a real change, but a benign and universal
 *    one, and refusing to live-edit every file authored on Windows would make
 *    the guard useless.
 *  - A trailing newline. Serializers routinely add or drop the final one.
 *  - Soft-wrapped paragraph lines, per `joinSoftWrappedLines`.
 *  - Table delimiter dash counts, per `normalizeTableDelimiters`.
 *
 * Everything else — dropped links, escaped HTML, destroyed tables, merged
 * callouts, renormalized list markers — still counts as lossy.
 */
function normalizeForComparison(text: string): string {
  return normalizeTableDelimiters(
    joinSoftWrappedLines(text.replace(/\r\n/g, "\n").replace(/\n+$/, "")),
  );
}

/**
 * Whether `body` survives a parse/serialize cycle unchanged.
 *
 * `reserialize` is injected rather than imported so this stays pure and
 * unit-testable; production passes the TipTap-backed implementation from
 * `markdownRoundTrip.ts`.
 */
export function isRoundTripStable(
  body: string,
  reserialize: (body: string) => string,
): boolean {
  // An empty (or whitespace-only) note has nothing to corrupt. Serializers
  // disagree about what empty output looks like, so short-circuit.
  if (body.trim() === "") return true;

  let output: string;
  try {
    output = reserialize(body);
  } catch {
    // If we cannot even round-trip it, we certainly cannot autosave it.
    return false;
  }

  return normalizeForComparison(output) === normalizeForComparison(body);
}

export function classifyRoundTrip(
  body: string,
  reserialize: (body: string) => string,
): RoundTripStatus {
  return isRoundTripStable(body, reserialize) ? "stable" : "lossy";
}

/**
 * The view mode a freshly-opened note should use.
 *
 * Lossy notes open in source mode. The user can still switch to live preview
 * deliberately — the guard informs the default, it does not forbid the choice.
 */
export function initialViewModeFor(status: RoundTripStatus): "live" | "source" {
  return status === "stable" ? "live" : "source";
}
