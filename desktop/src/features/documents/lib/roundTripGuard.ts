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

/**
 * Differences that are cosmetic rather than corrupting, and are normalized away
 * before comparison:
 *
 *  - CRLF vs LF. Writing back LF is a real change, but a benign and universal
 *    one, and refusing to live-edit every file authored on Windows would make
 *    the guard useless.
 *  - A trailing newline. Serializers routinely add or drop the final one.
 */
function normalizeForComparison(text: string): string {
  return text.replace(/\r\n/g, "\n").replace(/\n+$/, "");
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
