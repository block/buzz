import { hexToBytes } from "@noble/hashes/utils.js";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";
import type { Event as NostrEvent, EventTemplate } from "nostr-tools/pure";

import { TEST_IDENTITIES } from "./bridge";

// =============================================================================
// Hard-dataset relay seeder — GUI read-model overhaul (Dawn's lane)
// =============================================================================
//
// Publishes REAL signed Nostr events through the relay ingest path
// (`POST /events`), never raw SQL. This is the load-bearing fidelity choice:
// `thread_metadata` (depth, root, reply counts) is computed AT INGEST
// (buzz-relay/src/handlers/ingest.rs). Eva's channel-window surface reads that
// metadata; a raw-SQL bulk load would bypass computation and hand the window
// surface empty/wrong summaries — a false green. See
// PLANS/GUI_OVERHAUL_TEST_HARNESS_DAWN.md.
//
// Transport auth: the test relay runs BUZZ_REQUIRE_AUTH_TOKEN=false
// (start-relay-for-tests.sh), so `POST /events` accepts a plain `X-Pubkey`
// header (bridge.rs verify_bridge_auth dev fallback). The EVENT is still fully
// signed — only the per-request NIP-98 auth envelope is skipped. `POST /events`
// ingests ONE event per request, so bulk seeding parallelizes with a bounded
// concurrency pool. Authors must be seeded channel members
// (setup-desktop-test-data.sh seeds tyler/alice/bob/charlie into `general`),
// which `enforce_relay_membership` requires.
//
// Canonical tag shapes (crates/buzz-sdk/src/builders.rs thread_tags + ingest):
//   top-level : ["h", channelId]                       — no e-tag → depth NULL
//   direct    : ["e", parentId, "", "reply"]           — reply alone; root=parent
//   nested    : ["e", rootId, "", "root"],
//               ["e", parentId, "", "reply"]           — depth N
//   reaction  : kind 7, ["e", targetId], ["h", channelId], content = emoji
//   deletion  : kind 5, ["e", targetId]
// A reply carrying ONLY ["e", id, "", "root"] (no "reply" marker) is stored
// WITHOUT thread_metadata (ingest.rs returns None) — the "legacy/spurious row"
// shape used to probe contract-v1.1 item 7 (`depth IS NULL` = top-level).

const KIND_MESSAGE = 9;
const KIND_REACTION = 7;
const KIND_DELETION = 5;

const DEFAULT_RELAY_HTTP =
  process.env.BUZZ_E2E_RELAY_URL ?? "http://localhost:3000";

type IdentityName = keyof typeof TEST_IDENTITIES;

export type SeededEvent = {
  id: string;
  kind: number;
  pubkey: string;
  created_at: number;
  content: string;
  tags: string[][];
};

/** A signer bound to one seeded identity. */
class Signer {
  readonly pubkey: string;
  private readonly sk: Uint8Array;

  constructor(privateKeyHex: string) {
    this.sk = hexToBytes(privateKeyHex);
    this.pubkey = getPublicKey(this.sk);
  }

  sign(template: EventTemplate): NostrEvent {
    return finalizeEvent(template, this.sk);
  }
}

const signerCache = new Map<string, Signer>();

function signerFor(name: IdentityName): Signer {
  const cached = signerCache.get(name);
  if (cached) return cached;
  const signer = new Signer(TEST_IDENTITIES[name].privateKey);
  signerCache.set(name, signer);
  return signer;
}

export type SeedOptions = {
  relayHttpUrl?: string;
  /** Max in-flight POST /events requests. */
  concurrency?: number;
};

/**
 * POST one signed event through the relay ingest path. Throws on non-2xx so a
 * broken seed fails loudly rather than producing a silently-partial dataset.
 */
async function publishEvent(
  event: NostrEvent,
  relayHttpUrl: string,
): Promise<void> {
  const response = await fetch(`${relayHttpUrl}/events`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      // Dev-mode transport auth; the event body is fully signed regardless.
      "X-Pubkey": event.pubkey,
    },
    body: JSON.stringify(event),
  });
  if (!response.ok) {
    const detail = await response.text().catch(() => "<no body>");
    throw new Error(
      `POST /events failed (${response.status}) for kind ${event.kind} ${event.id.slice(0, 8)}: ${detail}`,
    );
  }
}

/**
 * Publish a batch with bounded concurrency, preserving ORDER GUARANTEES the
 * caller encodes as `barrier` boundaries: events within one barrier group may
 * publish concurrently, but a group only starts once every earlier group has
 * fully landed. Thread replies MUST NOT race their parents — ingest hard-errors
 * on an unknown parent (ingest.rs "reply parent not found") — so parents go in
 * an earlier group than their children.
 */
async function publishGroups(
  groups: NostrEvent[][],
  relayHttpUrl: string,
  concurrency: number,
): Promise<void> {
  for (const group of groups) {
    let cursor = 0;
    const workers: Promise<void>[] = [];
    const worker = async () => {
      while (cursor < group.length) {
        const event = group[cursor];
        cursor += 1;
        await publishEvent(event, relayHttpUrl);
      }
    };
    for (let i = 0; i < Math.min(concurrency, group.length); i += 1) {
      workers.push(worker());
    }
    await Promise.all(workers);
  }
}

// ── Event builders (canonical tag shapes) ────────────────────────────────────

function buildMessage(
  signer: Signer,
  channelId: string,
  content: string,
  createdAt: number,
  extraTags: string[][] = [],
): NostrEvent {
  return signer.sign({
    kind: KIND_MESSAGE,
    content,
    created_at: createdAt,
    tags: [["h", channelId], ...extraTags],
  });
}

function directReplyTags(parentId: string): string[][] {
  return [["e", parentId, "", "reply"]];
}

function nestedReplyTags(rootId: string, parentId: string): string[][] {
  return [
    ["e", rootId, "", "root"],
    ["e", parentId, "", "reply"],
  ];
}

/** The malformed "root-only" reply shape — no `reply` marker → no metadata. */
function legacyRootOnlyTags(rootId: string): string[][] {
  return [["e", rootId, "", "root"]];
}

function buildReaction(
  signer: Signer,
  channelId: string,
  targetId: string,
  emoji: string,
  createdAt: number,
): NostrEvent {
  return signer.sign({
    kind: KIND_REACTION,
    content: emoji,
    created_at: createdAt,
    tags: [
      ["e", targetId],
      ["h", channelId],
    ],
  });
}

function buildDeletion(
  signer: Signer,
  targetId: string,
  createdAt: number,
): NostrEvent {
  return signer.sign({
    kind: KIND_DELETION,
    content: "",
    created_at: createdAt,
    tags: [["e", targetId]],
  });
}

const toRow = (e: NostrEvent): SeededEvent => ({
  id: e.id,
  kind: e.kind,
  pubkey: e.pubkey,
  created_at: e.created_at,
  content: e.content,
  tags: e.tags,
});

const AUTHORS: IdentityName[] = ["tyler", "alice", "bob", "charlie"];

// ── Scenario builders ────────────────────────────────────────────────────────
//
// Each scenario returns { groups, expected }:
//  - groups: ordered barrier groups for publishGroups (parents before children)
//  - expected: the ground-truth SeededEvent[] the correctness suite asserts the
//    GUI must render (or, for aux/deleted, reason about) — the "every event the
//    relay returns must render" contract.

export type Scenario = {
  name: string;
  groups: NostrEvent[][];
  expected: SeededEvent[];
};

/**
 * Dense same-second wall: `count` top-level messages all at ONE created_at.
 * The exact keyset hazard — a bare `until` cursor can never advance past a
 * single second holding more rows than one page. All independent → one group.
 */
export function denseSecondWall(opts: {
  channelId: string;
  second: number;
  count: number;
}): Scenario {
  const { channelId, second, count } = opts;
  const events: NostrEvent[] = [];
  for (let i = 0; i < count; i += 1) {
    const signer = signerFor(AUTHORS[i % AUTHORS.length]);
    events.push(buildMessage(signer, channelId, `dense ${i}`, second));
  }
  return {
    name: "dense-second-wall",
    groups: [events],
    expected: events.map(toRow),
  };
}

/**
 * A single root with a `depth`-deep linear reply chain. Each level references
 * the previous as parent → must publish level-by-level (barrier per level), or
 * ingest rejects the child before its parent lands.
 */
export function deepThread(opts: {
  channelId: string;
  depth: number;
  startAt: number;
}): Scenario {
  const { channelId, depth, startAt } = opts;
  const root = buildMessage(
    signerFor("tyler"),
    channelId,
    "thread root",
    startAt,
    [],
  );
  const groups: NostrEvent[][] = [[root]];
  const expected: SeededEvent[] = [toRow(root)];
  let parentId = root.id;
  const rootId = root.id;
  for (let level = 1; level <= depth; level += 1) {
    const signer = signerFor(AUTHORS[level % AUTHORS.length]);
    const tags =
      level === 1 ? directReplyTags(rootId) : nestedReplyTags(rootId, parentId);
    const reply = buildMessage(
      signer,
      channelId,
      `reply depth ${level}`,
      startAt + level,
      tags,
    );
    groups.push([reply]);
    expected.push(toRow(reply));
    parentId = reply.id;
  }
  return { name: "deep-thread", groups, expected };
}

/**
 * Backdated events: `created_at` older than publish order — the author-clock
 * hazard that broke the created_at-anchored pager. Independent tops → one group.
 */
export function backdated(opts: {
  channelId: string;
  now: number;
  count: number;
}): Scenario {
  const { channelId, now, count } = opts;
  const events: NostrEvent[] = [];
  for (let i = 0; i < count; i += 1) {
    // Publish newest-first but stamp them progressively OLDER: the wire arrival
    // order and the created_at order deliberately disagree.
    const createdAt = now - (i + 1) * 37;
    const signer = signerFor(AUTHORS[i % AUTHORS.length]);
    events.push(
      buildMessage(
        signer,
        channelId,
        `backdated ${i} @${createdAt}`,
        createdAt,
      ),
    );
  }
  return { name: "backdated", groups: [events], expected: events.map(toRow) };
}

/**
 * Aux transitive closure (contract v1.1 item 1): a top-level row, a reaction on
 * it (aux), an edit-style follow, a deletion of the ROW, and — the two-hop case
 * — a DELETION OF THE REACTION. The window surface must return the reaction's
 * deletion even though it targets an aux id, not a row.
 */
export function auxClosure(opts: {
  channelId: string;
  startAt: number;
}): Scenario {
  const { channelId, startAt } = opts;
  const row = buildMessage(
    signerFor("tyler"),
    channelId,
    "aux target row",
    startAt,
  );
  const reaction = buildReaction(
    signerFor("alice"),
    channelId,
    row.id,
    "🔥",
    startAt + 1,
  );
  const groups: NostrEvent[][] = [[row]];
  const expected: SeededEvent[] = [toRow(row)];
  groups.push([reaction]);
  expected.push(toRow(reaction));
  // Two-hop: delete the reaction (targets an aux id, not the row).
  const reactionDeletion = buildDeletion(
    signerFor("alice"),
    reaction.id,
    startAt + 2,
  );
  groups.push([reactionDeletion]);
  expected.push(toRow(reactionDeletion));
  return { name: "aux-closure", groups, expected };
}

/**
 * Legacy/spurious-row probe (contract v1.1 item 7): a reply carrying only a
 * `root` marker (no `reply`) → stored WITHOUT thread_metadata. The correctness
 * suite asserts whether it wrongly surfaces as a top-level row, which is the
 * signal for whether a backfill migration is required.
 */
export function legacyReply(opts: {
  channelId: string;
  startAt: number;
}): Scenario {
  const { channelId, startAt } = opts;
  const root = buildMessage(
    signerFor("tyler"),
    channelId,
    "legacy root",
    startAt,
  );
  const spurious = buildMessage(
    signerFor("bob"),
    channelId,
    "legacy reply (root-only marker)",
    startAt + 1,
    legacyRootOnlyTags(root.id),
  );
  return {
    name: "legacy-reply",
    groups: [[root], [spurious]],
    expected: [toRow(root), toRow(spurious)],
  };
}

/**
 * Bulk mixed channel (3k+ events): a long span of top-level messages, a fraction
 * carrying short reply chains, reactions and a few deletions sprinkled across
 * rendered rows so `settled` (aux backfill committed — Sami's metric) reflects a
 * realistic fan-out cost, not a bare timeline. Groups: all roots + tops first
 * (independent), then replies (parents exist), then aux (targets exist).
 */
export function bulkChannel(opts: {
  channelId: string;
  topLevelCount: number;
  startAt: number;
  /** Fraction of top-level rows that get a 2-3 reply chain. */
  threadedFraction?: number;
  /** Fraction of rows that get a reaction. */
  reactionFraction?: number;
}): Scenario {
  const {
    channelId,
    topLevelCount,
    startAt,
    threadedFraction = 0.25,
    reactionFraction = 0.4,
  } = opts;

  const tops: NostrEvent[] = [];
  const replies: NostrEvent[] = [];
  const aux: NostrEvent[] = [];
  const expected: SeededEvent[] = [];

  for (let i = 0; i < topLevelCount; i += 1) {
    const at = startAt + i * 5; // spread across time, some collisions below
    const signer = signerFor(AUTHORS[i % AUTHORS.length]);
    // Every ~50th row shares a second with its neighbour — light dense pockets.
    const createdAt = i % 50 === 0 && i > 0 ? at - 5 : at;
    const top = buildMessage(signer, channelId, `msg ${i}`, createdAt);
    tops.push(top);
    expected.push(toRow(top));

    if (i % Math.max(1, Math.round(1 / threadedFraction)) === 0) {
      const chainLen = 2 + (i % 2); // 2 or 3 deep
      let parentId = top.id;
      const rootId = top.id;
      for (let level = 1; level <= chainLen; level += 1) {
        const rSigner = signerFor(AUTHORS[(i + level) % AUTHORS.length]);
        const tags =
          level === 1
            ? directReplyTags(rootId)
            : nestedReplyTags(rootId, parentId);
        const reply = buildMessage(
          rSigner,
          channelId,
          `msg ${i} reply ${level}`,
          createdAt + level,
          tags,
        );
        replies.push(reply);
        expected.push(toRow(reply));
        parentId = reply.id;
      }
    }

    if (i % Math.max(1, Math.round(1 / reactionFraction)) === 0) {
      const rSigner = signerFor(AUTHORS[(i + 1) % AUTHORS.length]);
      const reaction = buildReaction(
        rSigner,
        channelId,
        top.id,
        i % 3 === 0 ? "👍" : "🎉",
        createdAt + 1,
      );
      aux.push(reaction);
      expected.push(toRow(reaction));
    }
  }

  // Barrier ordering: tops must land before any reply; a depth-2 reply's parent
  // is a depth-1 reply, so replies split by depth into successive groups. A
  // depth-1 reply carries only a `reply` marker; deeper replies also carry a
  // `root` marker (nestedReplyTags). aux targets tops, so it goes last.
  const hasRootMarker = (e: NostrEvent) =>
    e.tags.some((t) => t[0] === "e" && t[3] === "root");
  const depth1 = replies.filter((r) => !hasRootMarker(r));
  const deeper = replies.filter((r) => hasRootMarker(r));

  const groups: NostrEvent[][] = [tops];
  if (depth1.length) groups.push(depth1);
  if (deeper.length) groups.push(deeper);
  if (aux.length) groups.push(aux);

  return { name: "bulk-channel", groups, expected };
}

/**
 * Exact-multiple final page (Eva 2026-07-03, `39006` window-bounds authority):
 * a channel with EXACTLY `pages * limitRows` top-level rows. The client must page
 * to the end without (a) a phantom extra page or (b) a terminal empty query —
 * `has_more` must go false on the page that returns the last full batch, driven by
 * the server `limit_rows+1` probe, not a "<limit means done" guess. This is the
 * boundary case that broke the first exhaustion ruling; it earns a permanent
 * regression. Deterministic distinct seconds so ordering is unambiguous.
 */
export function exactMultiplePage(opts: {
  channelId: string;
  limitRows: number;
  pages: number;
  startAt: number;
}): Scenario {
  const { channelId, limitRows, pages, startAt } = opts;
  const total = limitRows * pages;
  const events: NostrEvent[] = [];
  for (let i = 0; i < total; i += 1) {
    const signer = signerFor(AUTHORS[i % AUTHORS.length]);
    events.push(buildMessage(signer, channelId, `exact ${i}`, startAt + i));
  }
  return {
    name: "exact-multiple-page",
    groups: [events],
    expected: events.map(toRow),
  };
}

/**
 * Publish an already-built scenario, returning the ground-truth expected set.
 */
export async function seedScenario(
  scenario: Scenario,
  options: SeedOptions = {},
): Promise<SeededEvent[]> {
  const relayHttpUrl = options.relayHttpUrl ?? DEFAULT_RELAY_HTTP;
  const concurrency = options.concurrency ?? 16;
  await publishGroups(scenario.groups, relayHttpUrl, concurrency);
  return scenario.expected;
}

export const _internal = {
  buildMessage,
  buildReaction,
  buildDeletion,
  directReplyTags,
  nestedReplyTags,
  legacyRootOnlyTags,
  signerFor,
  publishGroups,
};
