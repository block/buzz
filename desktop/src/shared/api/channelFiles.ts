import { getChannelMessagesBefore } from "./tauriChannels";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";
import { linkChannelFileVersions as linkChannelFileVersionsTauri } from "./tauri";
import type { ChannelPageCursor, RelayEvent } from "./types";

/** Nostr message kinds that carry channel content (mirrors `TIMELINE_KINDS` in
 * `desktop/src-tauri/src/commands/messages.rs`, which `getChannelMessagesBefore`
 * queries server-side). */
const KIND_STREAM_MESSAGE = 9;
const KIND_STREAM_MESSAGE_V2 = 40002;

/** One keyset page's worth of events, capped at the relay's max (see
 * `get_channel_messages_before`'s `limit.unwrap_or(200).min(500)`). */
const PAGE_SIZE = 500;

/** A page count high enough to cover any realistically-sized channel without
 * looping forever if the relay ever returns a malformed cursor. */
const MAX_PAGES = 200;

/**
 * One file shared in a channel, with its version-chain links.
 *
 * Mirrors `ChannelFileEntry` in `crates/buzz-relay/src/api/files.rs`
 * (`#[serde(rename_all = "camelCase")]` on the Rust side, so the JSON keys
 * already land camelCase here — no mapping layer needed) and
 * `desktop/src-tauri/src/commands/channel_files.rs`'s copy of the same
 * struct. Keep all three in sync if this shape changes.
 */
export type ChannelFileEntry = {
  eventId: string;
  uploadedBy: string;
  /** Unix seconds. */
  uploadedAt: number;
  filename: string | null;
  sha256: string | null;
  size: number | null;
  mime: string | null;
  /** The imeta `url` tag verbatim — same value FileCard/FilePreviewModal use. */
  url: string | null;
  /** event_id of the file this one was tagged as a new version of, if any. */
  supersedes: string | null;
  /** event_id of a later upload tagged as superseding this one, if any. */
  supersededBy: string | null;
};

/** Extract the referenced event id of an `["e", "<id>", "<relay>",
 * "supersedes"]` tag, if present. Mirrors `supersedes_target` in the
 * now-unused `crates/buzz-relay/src/api/files.rs` — same tag shape, same
 * marker string, just read client-side instead of relay-side. */
function supersedesTarget(tags: string[][]): string | null {
  for (const tag of tags) {
    if (tag[0] !== "e") continue;
    const id = tag[1];
    const marker = tag[3];
    if (id && marker === "supersedes") return id;
  }
  return null;
}

/** Marker on the "which file is newer" side of a retroactive link-declaration
 * event — see `supersedesLinkDeclaration`. */
const SUPERSEDES_SUBJECT_MARKER = "supersedes-subject";

/**
 * Detect a retroactive "file B supersedes file A" link-declaration event:
 * no `imeta` tag of its own, one `e` tag marked `supersedes-subject` (the
 * newer file's event id) and one `e` tag marked `supersedes` (the older
 * file's event id). Published by `linkChannelFileVersions` (via the Rust
 * `link_channel_file_versions` command / `build_supersedes_link` builder) for
 * two files that were already sent before either upload's message carried a
 * `supersedes` tag of its own — Nostr events are immutable, so a later link
 * can't be added to either original event and instead rides a brand-new,
 * otherwise-empty event.
 *
 * Returns the `{subject, target}` event ids the caller merges into the same
 * `supersedes`/`supersededBy` graph built from own-message tags, or null if
 * `tags` doesn't carry both markers.
 */
function supersedesLinkDeclaration(
  tags: string[][],
): { subject: string; target: string } | null {
  let subject: string | null = null;
  let target: string | null = null;
  for (const tag of tags) {
    if (tag[0] !== "e") continue;
    const id = tag[1];
    const marker = tag[3];
    if (!id) continue;
    if (marker === SUPERSEDES_SUBJECT_MARKER) subject = id;
    else if (marker === "supersedes") target = id;
  }
  return subject && target ? { subject, target } : null;
}

/**
 * List every file shared in a channel (top-level messages only — see caveat
 * below), newest upload first.
 *
 * Deliberately does NOT call `list_channel_files` / `GET
 * /api/channels/{id}/files`: that custom relay endpoint only exists on a
 * self-hosted fork. Communities hosted on Block's BuilderLab service
 * (`*.communities.buzz.xyz`) run Block's own stock relay build, which has no
 * knowledge of this endpoint — calling it 404s for every BuilderLab-hosted
 * community, which is most of them.
 *
 * Instead this pages backward through the channel's full history via
 * `getChannelMessagesBefore` (the same `/query` bridge endpoint —
 * `POST {relay}/query` with a plain NIP-01 filter — that
 * `get_channel_messages_before`/`get_forum_posts` already depend on; no
 * fork-only relay code involved) and extracts every `imeta`-bearing message
 * client-side with `parseImetaTags`, the same parser message rendering
 * already uses for attachment cards. `supersedes`/`supersededBy` linkage is
 * reimplemented client-side too, mirroring the (now-unused) server-side
 * logic in `crates/buzz-relay/src/api/files.rs` exactly (same tag shape,
 * same two-pass approach: resolve `supersedes` per file, then back-fill
 * `supersededBy` from the resulting links).
 *
 * Caveat: `getChannelMessagesBefore` queries `TIMELINE_KINDS`, which the
 * relay scopes to *top-level* channel messages (thread replies are excluded
 * via a `thread_metadata` join) — so a file attached only inside a thread
 * reply won't show up here. Fine for a first pass; would need a separate
 * per-thread sweep (e.g. `getThreadReplies`) to close that gap.
 *
 * `crates/buzz-relay/src/api/files.rs` and the `list_channel_files` Tauri
 * command are NOT deleted — they still matter for anyone who self-hosts —
 * but the app must not depend on them being present.
 */
export async function listChannelFiles(
  channelId: string,
): Promise<ChannelFileEntry[]> {
  const events: RelayEvent[] = [];
  let cursor: ChannelPageCursor | null = null;

  for (let page = 0; page === 0 || (cursor && page < MAX_PAGES); page += 1) {
    const response = await getChannelMessagesBefore(
      channelId,
      cursor,
      PAGE_SIZE,
    );
    events.push(...response.events);
    cursor = response.nextCursor;
  }

  const files: ChannelFileEntry[] = [];
  // Retroactive links declared by a separate event (no imeta of its own) —
  // `subject` event id -> `target` (superseded) event id. Merged into the
  // same map as own-message `supersedes` tags below.
  const linkDeclarations: { subject: string; target: string }[] = [];
  for (const event of events) {
    if (
      event.kind !== KIND_STREAM_MESSAGE &&
      event.kind !== KIND_STREAM_MESSAGE_V2
    ) {
      continue;
    }
    const imetaEntries = parseImetaTags(event.tags);
    if (imetaEntries.size === 0) {
      const declaration = supersedesLinkDeclaration(event.tags);
      if (declaration) linkDeclarations.push(declaration);
      continue; // not a file-bearing message
    }
    const supersedes = supersedesTarget(event.tags);
    for (const entry of imetaEntries.values()) {
      files.push({
        eventId: event.id,
        uploadedBy: event.pubkey,
        uploadedAt: event.created_at,
        filename: entry.filename ?? null,
        sha256: entry.x ?? null,
        size: Number.isFinite(entry.size) ? entry.size : null,
        mime: entry.m ?? null,
        url: entry.url ?? null,
        supersedes,
        supersededBy: null, // back-filled below
      });
    }
  }

  // Build the newer-eventId -> older-eventId map from both sources: a file's
  // own `supersedes` tag (live-composer case) takes priority; a retroactive
  // link declaration only fills in an id that isn't already covered by an
  // own-tag link, so it can't silently override a file's own stated link.
  const supersedesByEventId = new Map<string, string>();
  for (const file of files) {
    if (file.supersedes) supersedesByEventId.set(file.eventId, file.supersedes);
  }
  for (const { subject, target } of linkDeclarations) {
    if (!supersedesByEventId.has(subject)) {
      supersedesByEventId.set(subject, target);
    }
  }

  // Second pass: apply the merged map back onto each file's `supersedes`
  // (covers the retroactive case, whose own tag didn't carry one) and
  // back-fill `supersededBy` now that the full set is known.
  for (const file of files) {
    const merged = supersedesByEventId.get(file.eventId);
    if (merged) file.supersedes = merged;
  }
  for (const file of files) {
    for (const [newerId, olderId] of supersedesByEventId) {
      if (olderId === file.eventId) {
        file.supersededBy = newerId;
        break;
      }
    }
  }

  return files;
}

/**
 * Retroactively declare that the file attached to `newerEventId` supersedes
 * the file attached to `olderEventId`. Thin wrapper around the Tauri
 * `link_channel_file_versions` command (`build_supersedes_link` on the Rust
 * side) — see `supersedesLinkDeclaration` above for the event shape this
 * produces and how `listChannelFiles` reads it back.
 */
export async function linkChannelFileVersions(
  channelId: string,
  newerEventId: string,
  olderEventId: string,
): Promise<void> {
  await linkChannelFileVersionsTauri(channelId, newerEventId, olderEventId);
}

/** True if `file` has since been superseded by a newer upload. */
export function isOutdatedFile(file: ChannelFileEntry): boolean {
  return file.supersededBy != null;
}
