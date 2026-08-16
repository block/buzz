/**
 * Tag markers for the file-versioning ("supersedes") graph.
 *
 * Deliberately a dependency-free leaf module. Both `channelFiles.ts` (which
 * builds the version graph) and `formatTimelineMessages.ts` (which must hide
 * link events from the timeline) need these, and routing the second through
 * the first would drag the whole channel-file-listing path — and its Tauri
 * imports — into the message renderer just for one predicate.
 */

/** Marker on the older file in an `["e", "<id>", "", "supersedes"]` tag. */
export const SUPERSEDES_MARKER = "supersedes";

/**
 * Marker on the newer file of a retroactive link-declaration event — see
 * `isSupersedesLinkDeclaration`.
 */
export const SUPERSEDES_SUBJECT_MARKER = "supersedes-subject";

/**
 * True if `tags` belong to a retroactive version-link event.
 *
 * `build_supersedes_link` (Rust) publishes these as **kind:9 with empty
 * content** — the same kind as an ordinary chat message — because
 * `listChannelFiles` can only discover them via the timeline query, which the
 * relay restricts to `TIMELINE_KINDS`. The side effect is that, absent an
 * explicit filter, tagging one file as a new version of another posts a blank
 * message to the channel under the tagger's name.
 *
 * Keying off the *subject* marker alone is deliberate and sufficient: nothing
 * but `build_supersedes_link` ever emits `supersedes-subject`, whereas a bare
 * `supersedes` marker also rides on ordinary file-upload messages, which must
 * keep rendering normally.
 */
export function isSupersedesLinkDeclaration(tags?: string[][] | null): boolean {
  return Boolean(
    tags?.some((tag) => tag[0] === "e" && tag[3] === SUPERSEDES_SUBJECT_MARKER),
  );
}
