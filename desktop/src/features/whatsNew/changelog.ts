/**
 * "What's new" changelog content, grouped by the `DEV_BUILD_LABEL` version it
 * shipped in (see `@/shared/lib/devBuildLabel`). Ordered ascending — append a
 * new `{version, bullets}` entry and bump `DEV_BUILD_LABEL` to surface it;
 * nothing else needs to change.
 *
 * `version` matches the trailing `vN` suffix of `DEV_BUILD_LABEL` (e.g.
 * `DEV_BUILD_LABEL = "k2v3"` corresponds to `version: "v3"` here).
 */
export type ChangelogEntry = {
  version: string;
  bullets: string[];
};

export const WHATS_NEW_CHANGELOG: ChangelogEntry[] = [
  {
    version: "v2",
    bullets: [
      "Native in-app file viewer for PDF, Word, Excel, and PowerPoint attachments",
    ],
  },
  {
    version: "v3",
    bullets: [
      "Files tab showing every file shared in a channel, with automatic and manual version tracking (mark outdated files when a newer version is shared)",
      "Higher-fidelity PowerPoint previews using LibreOffice when it's installed on your machine",
    ],
  },
  {
    version: "v4",
    bullets: [
      "Pin up to 3 important messages to the top of a channel or DM",
      "Forward one or more messages to other people or channels",
      "Clearer unread indicators for channels and DMs in the sidebar",
    ],
  },
];

function parseTrailingVersionNumber(value: string): number | null {
  const match = /v(\d+)$/i.exec(value.trim());
  if (!match) return null;
  const parsed = Number.parseInt(match[1], 10);
  return Number.isFinite(parsed) ? parsed : null;
}

/**
 * Changelog entries whose version is at or before `buildLabel` (e.g.
 * `buildLabel = "k2v3"` includes the `v2` and `v3` entries but not `v4`).
 * Returns an empty array if `buildLabel` is null or doesn't carry a
 * recognizable `vN` suffix.
 */
export function changelogEntriesUpToLabel(
  buildLabel: string | null,
): ChangelogEntry[] {
  if (!buildLabel) return [];
  const currentVersionNumber = parseTrailingVersionNumber(buildLabel);
  if (currentVersionNumber === null) return [];

  return WHATS_NEW_CHANGELOG.filter((entry) => {
    const entryVersionNumber = parseTrailingVersionNumber(entry.version);
    return (
      entryVersionNumber !== null && entryVersionNumber <= currentVersionNumber
    );
  });
}
