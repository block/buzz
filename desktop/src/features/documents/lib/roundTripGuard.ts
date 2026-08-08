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

import { parseCallout } from "@/features/documents/lib/obsidianSyntax";

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
/**
 * Canonicalises `*` and `+` bullet markers to `-`, including inside
 * blockquotes.
 *
 * All three are the same list in CommonMark; the serializer emits `-`. Treating
 * the choice of marker as a content change flagged files that differ only in
 * typing habit.
 */
function normalizeBulletMarkers(text: string): string {
  return text
    .split("\n")
    .map((line) => line.replace(/^(\s*(?:>\s*)*)[*+](\s)/, "$1-$2"))
    .join("\n");
}

/** Two or more trailing spaces: a markdown hard break, and therefore content. */
const HARD_BREAK_SUFFIX = / {2,}$/;

/**
 * Drops a lone trailing space from each line.
 *
 * One trailing space is invisible, means nothing to any markdown renderer, and
 * is left behind constantly by ordinary typing. The serializer drops it, so
 * comparing bytes flagged whole files over a single character — this was the
 * only difference in one real 64-line note.
 *
 * Two or more trailing spaces are a hard break and stay untouched, so a file
 * that uses them keeps failing the guard until the editor can represent them.
 */
function stripLoneTrailingSpace(text: string): string {
  return text
    .split("\n")
    .map((line) =>
      HARD_BREAK_SUFFIX.test(line) ? line : line.replace(/ $/, ""),
    )
    .join("\n");
}

/** A blockquote line, capturing its `>` prefix and the content after it. */
const QUOTE_LINE = /^( {0,3}>\s?)(.*)$/;

/**
 * Joins soft-wrapped lines inside a blockquote, unless it is a callout.
 *
 * Prose inside a `>` block wraps just like prose outside one, and the
 * serializer joins it the same way. The exception is load-bearing: an Obsidian
 * callout puts its title on the first line and its body on the next, so joining
 * those two would change the rendered callout. Callouts must keep failing the
 * guard until an extension can round-trip them.
 */
function joinSoftWrappedQuotes(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  let inCallout = false;

  for (const line of lines) {
    const match = QUOTE_LINE.exec(line);
    if (!match) {
      inCallout = false;
      out.push(line);
      continue;
    }

    const [, , content] = match;
    const previous = out.at(-1);
    const previousMatch =
      previous === undefined ? null : QUOTE_LINE.exec(previous);

    if (!previousMatch) {
      // First line of a blockquote decides whether the whole block is a
      // callout and therefore off-limits for joining.
      inCallout = parseCallout(`> ${content}`) !== null;
      out.push(line);
      continue;
    }

    const canJoin =
      !inCallout &&
      content.trim() !== "" &&
      previousMatch[2].trim() !== "" &&
      !BLOCK_START.test(content) &&
      !BLOCK_START.test(previousMatch[2]) &&
      !/ {2}$/.test(previousMatch[2]);

    if (canJoin) {
      out[out.length - 1] =
        `${previousMatch[1]}${previousMatch[2]} ${content.trim()}`;
    } else {
      out.push(line);
    }
  }

  return out.join("\n");
}

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

/** A list item at any of the three markers CommonMark allows. */
const LIST_ITEM = /^ {0,3}(?:[-*+]|\d+[.)])\s/;
/** Any blockquote line, callout or not. */
const QUOTE_START = /^ {0,3}>/;
/** An opening or closing code fence. */
const FENCE = /^\s*(?:`{3,}|~{3,})/;

/**
 * Removes blank lines that only separate one block from the next.
 *
 * Writing a list or a paragraph directly under its heading, with no blank
 * line, is an extremely common habit — it is how every daily-note template in
 * the vault I measured is written. CommonMark parses it identically either way
 * and the serializer always emits the blank line, so byte comparison failed on
 * 250+ files over invisible vertical whitespace. Runs of several blank lines
 * collapse to one for the same reason.
 *
 * Two exceptions keep meaning intact:
 *
 *  - Between two list items, a blank line makes the list *loose*, which really
 *    does render differently (each item gains a `<p>`).
 *  - Between two blockquotes, a blank line is the only thing keeping them from
 *    merging into one quote.
 *
 * In both cases the blank lines are preserved, so a change there still fails
 * the guard. Fenced code is skipped entirely — blank lines are content there.
 */
function normalizeBlockSeparation(text: string): string {
  const lines = text.split("\n");
  const out: string[] = [];
  let inFence = false;

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i];
    if (FENCE.test(line)) {
      inFence = !inFence;
      out.push(line);
      continue;
    }
    if (inFence || line.trim() !== "") {
      out.push(line);
      continue;
    }

    // A run of blank lines. Collapse it, then decide whether one survives.
    let end = i;
    while (end < lines.length && lines[end].trim() === "") end += 1;
    i = end - 1;

    const previous = out.at(-1);
    const next = lines[end];
    // Blank lines at either end of the document separate nothing.
    if (previous === undefined || next === undefined) continue;

    const betweenBlocks = BLOCK_START.test(previous) || BLOCK_START.test(next);
    const sameFamily =
      (LIST_ITEM.test(previous) && LIST_ITEM.test(next)) ||
      (QUOTE_START.test(previous) && QUOTE_START.test(next));

    // Between two paragraphs a blank line is the only thing keeping them
    // apart, so it always survives — collapsed to one, since a longer run is
    // just vertical whitespace.
    if (!betweenBlocks || sameFamily) out.push("");
  }

  return out.join("\n");
}

/** A thematic break in any of its three spellings, e.g. `***`, `- - -`, `___`. */
const THEMATIC_BREAK = /^ {0,3}(?:(?:\*\s*){3,}|(?:-\s*){3,}|(?:_\s*){3,})$/;

/**
 * Canonicalises every thematic break to `---`.
 *
 * `***`, `___` and `---` are the same horizontal rule; the serializer emits
 * `---`. This was the second-largest source of failures in the measured vault,
 * behind block separation.
 */
function normalizeThematicBreaks(text: string): string {
  return text
    .split("\n")
    .map((line) => (THEMATIC_BREAK.test(line) ? "---" : line))
    .join("\n");
}

/** Any pipe-delimited table row, header, delimiter or body. */
const TABLE_ROW = /^ {0,3}\|.*\|\s*$/;
/** A GFM table delimiter row, e.g. `| :--- | ---: |`. */
const TABLE_DELIMITER_ROW = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/;

/**
 * Reduces every table row to `|cell|cell|` with cells trimmed.
 *
 * Column padding is how humans keep a table readable in source form, and dash
 * counts in the delimiter row are pure alignment; the serializer discards both
 * for a canonical `| --- |`. Neither is visible once rendered. Alignment colons
 * are content and survive, because they are part of the cell text being
 * trimmed rather than the padding around it.
 */
function normalizeTableRows(text: string): string {
  return text
    .split("\n")
    .map((line) => {
      if (!TABLE_ROW.test(line)) return line;
      const canonical = TABLE_DELIMITER_ROW.test(line)
        ? line.replace(/-{2,}/g, "---")
        : line;
      return canonical
        .trim()
        .split("|")
        .map((cell) => cell.trim())
        .join("|");
    })
    .join("\n");
}

/**
 * Rewrites `_em_` as `*em*` and `__strong__` as `**strong**`.
 *
 * CommonMark treats the two spellings as identical and the serializer emits
 * the asterisk form. This was the single largest remaining cause of failures
 * in the measured vault.
 *
 * The lookarounds implement CommonMark's intraword rule: an underscore only
 * delimits emphasis when it does not sit between two word characters. Without
 * them `_Generated by weekly_report.py_` pairs its first two underscores and
 * normalizes to something the parser never produces. `snake_case_name` is left
 * alone entirely, which is the same reason.
 *
 * Emphasis the editor *drops* still fails the guard, since the markers then
 * survive on one side only.
 */
const INTRAWORD_UNDERSCORE = /(?<=\w)_(?=\w)/;

function normalizeEmphasisMarkers(text: string): string {
  return text
    .replace(/(?<!\w)__([^\n]+?)__(?!\w)/g, "**$1**")
    .replace(
      new RegExp(
        `(?<!\\w)_((?:[^_\\n]|${INTRAWORD_UNDERSCORE.source})+)_(?!\\w)`,
        "g",
      ),
      "*$1*",
    );
}

/**
 * Collapses runs of spaces *inside* a line to one.
 *
 * Every markdown renderer collapses them, so two spaces between words are
 * invisible; the serializer emits one. Leading indentation is untouched (it is
 * structural) and so is trailing whitespace, which `stripLoneTrailingSpace`
 * has already classified as either nothing or a hard break.
 *
 * Fenced code is skipped, where run length really is content.
 */
function collapseInnerSpaces(text: string): string {
  let inFence = false;
  return text
    .split("\n")
    .map((line) => {
      if (FENCE.test(line)) {
        inFence = !inFence;
        return line;
      }
      return inFence ? line : line.replace(/(\S) {2,}(?=\S)/g, "$1 ");
    })
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
 *  - Soft-wrapped prose inside non-callout blockquotes.
 *  - `*` and `+` bullet markers, which mean the same as `-`.
 *  - A single trailing space, per `stripLoneTrailingSpace`.
 *  - Blank lines that only separate blocks, per `normalizeBlockSeparation`.
 *  - `***` and `___` thematic breaks, which mean the same as `---`.
 *  - Table column padding and delimiter dash counts, per `normalizeTableRows`.
 *  - `_em_` versus `*em*`, per `normalizeEmphasisMarkers`.
 *  - Runs of spaces inside a line, per `collapseInnerSpaces`.
 *
 * The property they share: **a reader cannot see any of them.** Differences a
 * reader *would* see — dropped links, escaped HTML, destroyed tables, merged
 * callouts, tight versus loose lists, two-space hard breaks — still count as
 * lossy and still send the file to source mode.
 *
 * Each entry is also a promise that saving may rewrite the file that way, which
 * shows up in `git diff` even though nothing rendered changed. That is the
 * deliberate trade: byte-exact comparison left live preview usable on 4% of a
 * real 470-note vault, which is indistinguishable from not shipping it.
 */
function normalizeForComparison(text: string): string {
  const base = collapseInnerSpaces(
    stripLoneTrailingSpace(text.replace(/\r\n/g, "\n").replace(/\n+$/, "")),
  );
  return normalizeEmphasisMarkers(
    normalizeTableRows(
      normalizeBulletMarkers(
        joinSoftWrappedQuotes(
          joinSoftWrappedLines(
            normalizeBlockSeparation(normalizeThematicBreaks(base)),
          ),
        ),
      ),
    ),
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
