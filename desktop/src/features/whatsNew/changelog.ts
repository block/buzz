/**
 * "What's new" changelog content, keyed by the app's full version string as
 * reported by `@tauri-apps/api/app`'s `getVersion()`. Append a new
 * `{version, bullets}` entry each time a release ships user-facing changes;
 * `useWhatsNewModal` picks it up automatically.
 *
 * Keyed by the whole string rather than the trailing `-N` on purpose. The
 * original scheme parsed that number and showed every entry `<= N`, which
 * broke the moment the fork rebased onto upstream 0.5.14 and the version went
 * `0.5.5-5` -> `0.5.14-0`: the counter reset to 0, no entry matched, and the
 * splash silently stopped appearing. Worse, it would have come back wrong —
 * `0.5.14-2` would have re-shown the year-old entry 2 as if it were new.
 * Array order is authoritative here, and nothing is derived from the numbers
 * inside the version string.
 *
 * The first three entries predate per-release tracking: they were splash
 * milestones bundled into this fork's first real release (`0.5.5-4`, before
 * the GitHub Actions pipeline existed), kept for continuity.
 */
export type ChangelogEntry = {
  /** Full app version, e.g. `"0.5.14-1"`. */
  version: string;
  bullets: string[];
};

export const WHATS_NEW_CHANGELOG: ChangelogEntry[] = [
  {
    version: "0.5.5-2",
    bullets: [
      "Native in-app file viewer for PDF, Word, Excel, and PowerPoint attachments",
    ],
  },
  {
    version: "0.5.5-3",
    bullets: [
      "Files tab showing every file shared in a channel, with automatic and manual version tracking (mark outdated files when a newer version is shared)",
      "Higher-fidelity PowerPoint previews using LibreOffice when it's installed on your machine",
    ],
  },
  {
    version: "0.5.5-4",
    bullets: [
      "Pin up to 3 important messages to the top of a channel or DM",
      "Forward one or more messages to other people or channels",
      "Clearer unread indicators for channels and DMs in the sidebar",
    ],
  },
  {
    version: "0.5.5-5",
    bullets: [
      "Start a Google Meet from any channel or DM — connect your Google account in Settings, then share a call link in one click",
    ],
  },
  {
    version: "0.5.14-0",
    bullets: [
      "Updated to the latest Buzz release, bringing nine versions of upstream improvements",
      "Fixed Windows desktop notifications not appearing at all",
    ],
  },
  {
    version: "0.5.14-1",
    bullets: [
      "File versions are now grouped: the current version shows in the Files tab with earlier ones tucked underneath",
      "Outdated files are marked wherever you see them — in chat, in the preview window, and in the Files tab — and one click takes you to the latest version",
      "Tagging a new version no longer posts a blank message to the channel",
      "Deleting a file now correctly clears its version link",
      'When you attach a file, Buzz suggests which existing file it might be a new version of, and now recognises names like "report-v2.pdf" and "deck FINAL.pptx"',
      "The Files tab shows upload times, so several versions shared on one day can be told apart",
      "Clearer unread dot on the channel you're currently reading",
    ],
  },
];

/**
 * The changelog entry for exactly `appVersion`, or null.
 *
 * Matched on the whole string; nothing is parsed out of it and no ordering
 * comparison is attempted. That is the point: `0.5.5-5` -> `0.5.14-0` sorts
 * backwards under every ordinary string or semver comparison while being
 * forwards in time, and the previous implementation — which derived the entry
 * from the trailing `-N` — silently stopped matching anything the moment the
 * fork rebased, disabling the splash for a whole release without a symptom.
 *
 * Null for a dev build or a release that shipped no user-facing change; the
 * caller simply shows no splash. Position in {@link WHATS_NEW_CHANGELOG} is
 * the release order for anything that needs it (see `ReleaseHistory`).
 */
export function changelogEntryForVersion(
  appVersion: string | null,
): ChangelogEntry | null {
  if (!appVersion) return null;
  const trimmed = appVersion.trim();
  return WHATS_NEW_CHANGELOG.find((entry) => entry.version === trimmed) ?? null;
}
