import { getChannelMessagesBefore } from "./tauriChannels";
import { getThreadReplies } from "./tauri";
import { collectChannelLinkEntries } from "@/shared/lib/channelLinkEntries.mjs";
import { parseImetaTags } from "@/shared/ui/markdown/parseImeta";
import { SUPERSEDES_MARKER, SUPERSEDES_SUBJECT_MARKER } from "./supersedesTags";
import type { ChannelPageCursor, RelayEvent, ThreadCursor } from "./types";

/** Nostr message kinds that carry channel content (mirrors `TIMELINE_KINDS` in
 * `desktop/src-tauri/src/commands/messages.rs`, which `getChannelMessagesBefore`
 * queries server-side). */
const KIND_STREAM_MESSAGE = 9;
const KIND_STREAM_MESSAGE_V2 = 40002;
/** Relay-emitted system message; carries the deletion tombstone below. */
const KIND_SYSTEM_MESSAGE = 40099;

/**
 * Event ids the relay has recorded as deleted, read from its kind:40099
 * tombstones (`{"type":"message_deleted","target_event_id":"<hex>"}` — see
 * `handle_delete_event` in `crates/buzz-relay/src/handlers/side_effects.rs`).
 *
 * This matters because the relay *soft*-deletes: a deleted message keeps
 * coming back from `/query`, so without reading tombstones a deleted file
 * stays in the Files tab forever and keeps asserting its version link —
 * verified on 0.5.14-0, where deleting either side of a pair left both badges
 * untouched.
 *
 * The tombstone kind is already in `TIMELINE_KINDS`, so these events arrive in
 * the same pages the file scan already walks; no extra query is needed.
 */
function deletedEventIds(events: RelayEvent[]): Set<string> {
  const deleted = new Set<string>();
  for (const event of events) {
    if (event.kind !== KIND_SYSTEM_MESSAGE) continue;
    let payload: { type?: string; target_event_id?: string };
    try {
      payload = JSON.parse(event.content) as typeof payload;
    } catch {
      continue; // not JSON — some other system row
    }
    if (payload.type !== "message_deleted") continue;
    if (payload.target_event_id) deleted.add(payload.target_event_id);
  }
  return deleted;
}

/** One keyset page's worth of events, capped at the relay's max (see
 * `get_channel_messages_before`'s `limit.unwrap_or(200).min(500)`). */
const PAGE_SIZE = 500;

/** A page count high enough to cover any realistically-sized channel without
 * looping forever if the relay ever returns a malformed cursor. */
const MAX_PAGES = 200;

/** One thread's reply page. `get_thread_replies` caps its own return, so this
 * only needs to be large enough that most threads resolve in a single page. */
const THREAD_PAGE_SIZE = 200;

/** Per-thread page ceiling — the same malformed-cursor guard as `MAX_PAGES`,
 * scaled down since a single thread is far smaller than a whole channel. */
const MAX_THREAD_PAGES = 50;

/**
 * How many threads to sweep at once.
 *
 * The relay exposes no "all replies in a channel" query — every channel-scope
 * query is forced `top_level` (see `build_channel_window_filter` /
 * `build_channel_messages_before_filter` in `commands`), so the only way to
 * reach files posted inside thread replies is to walk each thread with
 * `get_thread_replies`. That's one request per top-level message; a small pool
 * keeps the Files-tab load from firing hundreds of requests simultaneously
 * while still finishing quickly. Threads with no replies return an empty page
 * and cost one cheap round-trip.
 */
const THREAD_SWEEP_CONCURRENCY = 8;

/**
 * Fetch every reply event under the given thread roots.
 *
 * `get_thread_replies` returns the whole subtree for a root in one shot
 * (depth-bounded, paged only for very large threads), so one sweep per
 * top-level message reaches replies at any nesting depth without recursing.
 * Roots are swept through a bounded worker pool.
 */
async function collectThreadReplyEvents(
  channelId: string,
  rootEventIds: readonly string[],
): Promise<RelayEvent[]> {
  const collected: RelayEvent[] = [];
  let nextIndex = 0;

  async function worker(): Promise<void> {
    while (nextIndex < rootEventIds.length) {
      const rootEventId = rootEventIds[nextIndex];
      nextIndex += 1;

      // Best-effort per thread: a file inside a reply is an enhancement over
      // the top-level list, so one thread's fetch failing must not fail the
      // whole Files tab. Skip that thread and keep sweeping the rest.
      try {
        let cursor: ThreadCursor | null = null;
        for (
          let page = 0;
          page === 0 || (cursor && page < MAX_THREAD_PAGES);
          page += 1
        ) {
          const response = await getThreadReplies(rootEventId, channelId, {
            limit: THREAD_PAGE_SIZE,
            cursor,
          });
          collected.push(...response.events);
          cursor = response.nextCursor;
          if (!cursor) break;
        }
      } catch {
        // Ignore and move on to the next root.
      }
    }
  }

  const workerCount = Math.min(THREAD_SWEEP_CONCURRENCY, rootEventIds.length);
  await Promise.all(Array.from({ length: workerCount }, () => worker()));
  return collected;
}

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
  /**
   * `"link"` for a URL shared in a message body rather than an uploaded file.
   * Link entries sit in the same list — and the same version chains — because
   * the supersedes tag points at an *event*, not at a file, so a Drive link can
   * supersede an uploaded PDF with no new tag and no relay change. See
   * `shared/lib/channelLinkEntries.mjs`.
   */
  kind: "file" | "link";
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
    if (id && marker === SUPERSEDES_MARKER) return id;
  }
  return null;
}

/**
 * Detect a retroactive "file B supersedes file A" link-declaration event:
 * no `imeta` tag of its own, one `e` tag marked `supersedes-subject` (the
 * newer file's event id) and one `e` tag marked `supersedes` (the older
 * file's event id).
 *
 * READ-ONLY LEGACY PATH. Nothing publishes these any more: the builder and
 * command were removed because the event had to be kind:9 (the ordinary
 * message kind) to be discoverable here, which meant every version tag also
 * posted a blank message to the channel. Version links are now set only at
 * upload time, as a tag on the file's own message. This parser stays so links
 * published by earlier builds keep resolving.
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
    else if (marker === SUPERSEDES_MARKER) target = id;
  }
  return subject && target ? { subject, target } : null;
}

/**
 * List every file shared in a channel — including files posted inside thread
 * replies (see the thread-replies note below) — newest upload first.
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
 * Thread replies: `getChannelMessagesBefore` queries `TIMELINE_KINDS` scoped
 * to *top-level* channel messages (replies are excluded via a `thread_metadata`
 * join), so files posted inside a thread reply don't come back in those pages.
 * To include them, after paging the top-level history we sweep each top-level
 * message's thread with `getThreadReplies` (see `collectThreadReplyEvents`) and
 * fold the reply events into the same `events` array. Everything downstream —
 * deletion tombstones, imeta/link extraction, version chains — then treats a
 * reply-borne file exactly like a top-level one, so it lands in the Files tab
 * with full version and deletion handling.
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

  // Every top-level content message is a potential thread root. Sweep each
  // thread and fold its replies into `events` so files posted inside replies
  // are indexed alongside top-level ones. Done before deletion/extraction so a
  // reply file, a reply's version tag, and a reply-deleting tombstone are all
  // processed by the identical passes below.
  const threadRootEventIds = events
    .filter(
      (event) =>
        event.kind === KIND_STREAM_MESSAGE ||
        event.kind === KIND_STREAM_MESSAGE_V2,
    )
    .map((event) => event.id);
  const replyEvents = await collectThreadReplyEvents(
    channelId,
    threadRootEventIds,
  );
  // Defensive de-dup by id: reply subtrees never overlap and pages don't
  // repeat, but combining two independently paged sources shouldn't be able to
  // produce a duplicate file entry if either ever returns an id twice.
  const seenEventIds = new Set(events.map((event) => event.id));
  for (const replyEvent of replyEvents) {
    if (seenEventIds.has(replyEvent.id)) continue;
    seenEventIds.add(replyEvent.id);
    events.push(replyEvent);
  }

  // Collected first: a tombstone can appear anywhere in the paged stream
  // relative to the message it deletes, so the file scan below needs the full
  // set before it can decide what to skip.
  const deleted = deletedEventIds(events);

  const files: ChannelFileEntry[] = [];
  // Retroactive links declared by a separate event (no imeta of its own) —
  // `subject` event id -> `target` (superseded) event id. Merged into the
  // same map as own-message `supersedes` tags below.
  const linkDeclarations: { subject: string; target: string }[] = [];
  // Every surviving content event, for the link sweep below. Collected in the
  // same pass rather than a second walk over `events`, since the deletion and
  // kind filtering are identical.
  const linkSources: {
    eventId: string;
    pubkey: string;
    createdAt: number;
    content: string;
    hasAttachment: boolean;
    supersedes: string | null;
  }[] = [];
  for (const event of events) {
    if (
      event.kind !== KIND_STREAM_MESSAGE &&
      event.kind !== KIND_STREAM_MESSAGE_V2
    ) {
      continue;
    }
    // A deleted message contributes neither its file nor any version link it
    // asserted — deleting is the only way to undo a link, so it has to undo
    // the link too, not just hide the row. The same applies to a link it
    // carried: deleting the message is how you take a link back.
    if (deleted.has(event.id)) continue;
    const imetaEntries = parseImetaTags(event.tags);
    const supersedes = supersedesTarget(event.tags);
    linkSources.push({
      eventId: event.id,
      pubkey: event.pubkey,
      createdAt: event.created_at,
      content: event.content,
      hasAttachment: imetaEntries.size > 0,
      supersedes,
    });
    if (imetaEntries.size === 0) {
      const declaration = supersedesLinkDeclaration(event.tags);
      if (declaration) linkDeclarations.push(declaration);
      continue; // not a file-bearing message
    }
    for (const entry of imetaEntries.values()) {
      files.push({
        kind: "file",
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

  // Links shared in message bodies join the same list, so they get version
  // chains, deletion handling and Files-tab rendering for free. A URL that is
  // already an uploaded file's own URL is excluded: the markdown renderer
  // embeds an attachment's URL in the message body, so without this every
  // upload would also produce a duplicate link row beside itself.
  files.push(
    ...collectChannelLinkEntries({
      messages: linkSources,
      excludedUrls: files.map((file) => file.url),
    }),
  );

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
    // Files only. A message carrying both an attachment and a link shares one
    // event id between two entries, and the supersedes tag on it belongs to
    // the file — applying it here as well would give the link a "New version"
    // badge for a predecessor it never claimed. `collectChannelLinkEntries`
    // makes the same call at the other end.
    if (file.kind !== "file") continue;
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

  // Drop links whose other end is gone. A `supersedes` pointing at a deleted
  // (or never-fetched) file would otherwise render a "New version" badge for a
  // predecessor nobody can see — the exact symptom observed on 0.5.14-0, where
  // deleting the outdated file left the newer one still claiming to be a new
  // version of it.
  const presentEventIds = new Set(files.map((file) => file.eventId));
  for (const file of files) {
    if (file.supersedes && !presentEventIds.has(file.supersedes)) {
      file.supersedes = null;
    }
    if (file.supersededBy && !presentEventIds.has(file.supersededBy)) {
      file.supersededBy = null;
    }
  }

  return files;
}

/** True if `file` has since been superseded by a newer upload. */
export function isOutdatedFile(file: ChannelFileEntry): boolean {
  return file.supersededBy != null;
}

/**
 * Where a file sits in its channel's version chain.
 *
 * Lives here rather than beside the badge component so both consumers can
 * derive it from the same place: the Files tab reads it straight off a
 * `ChannelFileEntry`, while chat bubbles and the preview modal get it from
 * `FileVersionContext`, which only knows a URL.
 */
export type FileVersionStatus = {
  /** A later upload in this channel was tagged as superseding this file. */
  outdated: boolean;
  /** This file was itself tagged as a newer version of an earlier upload. */
  isNewVersion: boolean;
};

/** Derive the two version flags a badge renders from a file entry. */
export function fileVersionStatus(file: ChannelFileEntry): FileVersionStatus {
  return {
    outdated: isOutdatedFile(file),
    isNewVersion: file.supersedes != null,
  };
}
