/**
 * Shared strip helpers for Bestie outbound turn hints (live lists / list teach /
 * job teach). Kept free of store imports so NL apply paths can strip safely.
 */

import { BESTIE_JOB_TURN_HINT_MARKER } from "./bestieJobProtocol";
import { BESTIE_LIST_TURN_HINT_MARKER } from "./bestieListProtocol";

/** Marker for live RHS snapshot appended on every Bestie user send. */
export const BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER = "[Bestie live lists]";

const OUTBOUND_HINT_MARKERS = [
  BESTIE_LIVE_LIST_STATE_TURN_HINT_MARKER,
  BESTIE_LIST_TURN_HINT_MARKER,
  BESTIE_JOB_TURN_HINT_MARKER,
] as const;

/**
 * Strip live-list / list / job turn hints (and anything after the earliest
 * marker) for UI display and for NL intent parsers.
 */
export function stripBestieOutboundHints(
  content: string | null | undefined,
): string {
  // View-thread / activity rows can surface events before body decrypt; never
  // throw on undefined (content.indexOf TypeError).
  if (typeof content !== "string") return "";
  let earliest = -1;
  for (const marker of OUTBOUND_HINT_MARKERS) {
    const withBlank = content.indexOf(`\n\n${marker}`);
    if (withBlank >= 0 && (earliest < 0 || withBlank < earliest)) {
      earliest = withBlank;
      continue;
    }
    const at = content.indexOf(marker);
    if (at >= 0 && (at === 0 || content[at - 1] === "\n")) {
      const blank = content.lastIndexOf("\n\n", at);
      const cut =
        blank >= 0 && blank + 2 === at
          ? blank
          : at > 0 && content[at - 1] === "\n"
            ? at - 1
            : at;
      if (earliest < 0 || cut < earliest) earliest = cut;
    }
  }
  if (earliest < 0) return content;
  return content.slice(0, earliest).trimEnd();
}
