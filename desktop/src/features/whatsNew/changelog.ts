/**
 * "What's new" changelog content, grouped by the release's own pre-release
 * number — the trailing `-N` in the app's real version string (e.g.
 * `"0.5.5-5"` -> `5`), as reported by `@tauri-apps/api/app`'s `getVersion()`.
 * There's no separate build-label system anymore: append a new
 * `{version, bullets}` entry each time a release ships user-facing changes,
 * and `useWhatsNewModal` picks it up automatically by comparing against the
 * running app's actual version — nothing else needs to change.
 *
 * Historical entries 2-4 predate this scheme: they were three splash
 * milestones bundled into the single first real release this fork shipped
 * (`0.5.5-4`, before this repo had a working GitHub Actions release
 * pipeline), kept as-is for continuity. Every entry from 5 onward maps 1:1
 * to its release tag's trailing number.
 */
export type ChangelogEntry = {
  version: number;
  bullets: string[];
};

export const WHATS_NEW_CHANGELOG: ChangelogEntry[] = [
  {
    version: 2,
    bullets: [
      "Native in-app file viewer for PDF, Word, Excel, and PowerPoint attachments",
    ],
  },
  {
    version: 3,
    bullets: [
      "Files tab showing every file shared in a channel, with automatic and manual version tracking (mark outdated files when a newer version is shared)",
      "Higher-fidelity PowerPoint previews using LibreOffice when it's installed on your machine",
    ],
  },
  {
    version: 4,
    bullets: [
      "Pin up to 3 important messages to the top of a channel or DM",
      "Forward one or more messages to other people or channels",
      "Clearer unread indicators for channels and DMs in the sidebar",
    ],
  },
];

/**
 * Parses the trailing pre-release number off the app's own version string
 * (e.g. `"0.5.5-5"` -> `5`). Returns `null` if the string is missing or
 * doesn't carry a numeric `-N` suffix — shouldn't happen for a real
 * k2alpha build, but this guards against it instead of throwing.
 */
export function parseReleaseNumber(appVersion: string | null): number | null {
  if (!appVersion) return null;
  const match = /-(\d+)$/.exec(appVersion.trim());
  if (!match) return null;
  const parsed = Number.parseInt(match[1], 10);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * Changelog entries whose version is at or before `currentVersionNumber`
 * (e.g. `currentVersionNumber = 3` includes the `2` and `3` entries but not
 * `4`). Returns an empty array if `currentVersionNumber` is `null`.
 */
export function changelogEntriesUpToVersion(
  currentVersionNumber: number | null,
): ChangelogEntry[] {
  if (currentVersionNumber === null) return [];

  return WHATS_NEW_CHANGELOG.filter(
    (entry) => entry.version <= currentVersionNumber,
  );
}
