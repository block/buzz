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
  /**
   * Release date as `YYYY-MM-DD`, taken from the git tag.
   *
   * Only used to interleave these entries with upstream Buzz releases in the
   * Settings → Updates history. Optional because the two earliest entries
   * predate the release pipeline and were never tagged, so no date exists for
   * them; undated entries sort to the end, which is where they belong anyway.
   * Nothing else derives ordering from this — array position remains
   * authoritative for release order.
   */
  date?: string;
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
    date: "2026-08-12",
    bullets: [
      "Pin up to 3 important messages to the top of a channel or DM",
      "Forward one or more messages to other people or channels",
      "Clearer unread indicators for channels and DMs in the sidebar",
    ],
  },
  {
    version: "0.5.5-5",
    date: "2026-08-14",
    bullets: [
      "Start a Google Meet from any channel or DM — connect your Google account in Settings, then share a call link in one click",
    ],
  },
  {
    version: "0.5.14-0",
    date: "2026-08-15",
    bullets: [
      "Updated to the latest Buzz release, bringing nine versions of upstream improvements",
      "Fixed Windows desktop notifications not appearing at all",
    ],
  },
  {
    version: "0.5.14-1",
    date: "2026-08-15",
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
  {
    version: "0.5.14-2",
    date: "2026-08-16",
    bullets: [
      "Your Inbox now shows every channel and DM with something new, whether or not you were mentioned",
      "One row per conversation, with a count — not one row per message",
      "Reading a conversation clears it from the Inbox, and reading it in the Inbox clears it everywhere else",
      "Muted channels stay out of the Inbox",
    ],
  },
  {
    version: "0.5.14-3",
    date: "2026-08-16",
    bullets: [
      "The Inbox now shows only what is addressed to you: mentions, threads you are part of, and approvals waiting on you. Use the sidebar to see everything else that is new",
      "A mention inside a thread is counted once, as a mention, rather than appearing under both Mentions and Threads",
      "Threads now leave the Inbox once you have read them",
      "Settings → Updates lists upstream Buzz releases alongside this app's own, up to the version this build is based on",
    ],
  },
  {
    version: "0.5.14-4",
    date: "2026-08-17",
    bullets: [
      "The Inbox now shows only unread items by default; switch off 'Show unread only' to see everything again",
      "The conversation you have open stays in the list while you read it, now marked 'Viewing' so it is clear why it is still there",
      "Collapsed sidebar sections now show what is waiting inside them: a count when someone has mentioned you or sent a DM, a dot for ordinary activity, and nothing when the section is quiet",
    ],
  },
  // Repeats 0.5.14-4's bullets on purpose. The splash shows only the running
  // version's entry, so anyone updating 0.5.14-3 → 0.5.14-5 would otherwise
  // never see them — and -4 shipped hours before this, against a six-hourly
  // update check, so most people never ran it.
  {
    version: "0.5.14-5",
    date: "2026-08-17",
    bullets: [
      "Type @channel to notify everyone in a channel, or @here to notify only the people currently online",
      "@channel reaches people even in a channel they have muted; @here never does",
      "The Inbox now shows only unread items by default; switch off 'Show unread only' to see everything again",
      "The conversation you have open stays in the list while you read it, now marked 'Viewing' so it is clear why it is still there",
      "Collapsed sidebar sections now show what is waiting inside them: a count when someone has mentioned you or sent a DM, a dot for ordinary activity, and nothing when the section is quiet",
    ],
  },
  {
    version: "0.5.14-6",
    date: "2026-08-17",
    bullets: [
      "Updates now show real download progress instead of a label that never changed",
      "Release history in Settings → Updates lists same-day releases newest-first, as it always should have",
    ],
  },
  {
    version: "0.5.16-0",
    date: "2026-08-17",
    bullets: [
      "Updated to the latest Buzz release, bringing two versions of upstream improvements",
      "Agent mentions are preserved when the relay errors, and shared agent access is enforced across your devices",
      "Sidebar fixes: the collapsed sidebar no longer paints over the community rail, and preview rows line up correctly",
      "The Bumble agent is now called Pollen",
    ],
  },
  {
    version: "0.5.17-0",
    date: "2026-08-18",
    bullets: [
      "Buzz uses noticeably less CPU while sitting open, so laptops stay cooler and quieter",
      "Security update to a networking dependency",
      "Online/away status is more reliable, which also makes @here more accurate",
    ],
  },
  {
    version: "0.5.17-1",
    date: "2026-08-21",
    bullets: [
      "Video, audio and files over 5 MB now go to your own Google Drive and are shared as a link — no more ffmpeg, no more upload failures on large files",
      "Connect your Google account under Settings → Voice to use this. If you connected it before today, disconnect and reconnect once",
      "Links shared in a channel now appear in its Files tab, alongside uploaded files",
      "A link can be marked as a new version of a file, and a file as a new version of a link",
    ],
  },
  {
    version: "0.5.18-0",
    date: "2026-08-21",
    bullets: [
      "Desktop notifications now name who sent the message",
      "Font size and conversation density can be adjusted, with a live preview, under Settings",
      "Ctrl+W closes the window",
      "Faster message rendering, and several fixes: recent-channel ordering, emoji recents, and archived channels no longer appearing in # autocomplete",
    ],
  },
  {
    version: "0.5.18-1",
    date: "2026-08-21",
    bullets: [
      "Programs (.exe, .dll and similar) can now be shared — they go to your Google Drive instead of failing to upload",
    ],
  },
  {
    version: "0.5.18-2",
    date: "2026-08-24",
    bullets: [
      "Fixed Windows desktop notifications never appearing — new messages now show a toast, and clicking it opens the conversation",
    ],
  },
  {
    version: "0.5.18-3",
    date: "2026-08-24",
    bullets: [
      "Fixed Windows desktop notifications — new messages now show a native toast, clicking it opens the conversation, and Buzz is registered in Windows Notification Settings on launch",
    ],
  },
  {
    version: "0.5.20-1",
    date: "2026-08-26",
    bullets: [
      "Desktop notifications now alert for every message in a channel until you mute it, and direct messages and thread replies alert reliably regardless of your sound settings",
      "Type @channel or @here in the composer to reach everyone or just those currently active — now offered as autocomplete suggestions",
      "Caught up to upstream Buzz 0.5.20: faster channel switching, image navigation across message threads, and a range of composer and stability fixes",
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
