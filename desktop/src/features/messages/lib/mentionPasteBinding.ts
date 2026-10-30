import * as React from "react";

import { trimMapToSize } from "@/shared/lib/trimMapToSize";

import {
  canonicalMentionLabel,
  parseMentionClipboardRecords,
  selectVisibleMentionIdentities,
  selectVouchedMentionIdentities,
  type MentionIdentity,
  type VerifyMentionIdentities,
} from "./mentionClipboard";
import { findMentionTokenSpans } from "./mentionTokenSpans";
import {
  MAX_TRACKED_OCCURRENCES,
  readPastedMentionOccurrenceRange,
  releasePastedMentionOccurrence,
  trackPastedMentionOccurrence,
  type PastedMentionOccurrenceView,
} from "./pastedMentionOccurrences";

/** Writes one `name → pubkey` pair into the composer's mention map. */
export type RegisterMentionPubkey = (
  displayName: string,
  pubkey: string,
  options?: { isAgent?: boolean },
) => void;

/** What a composer's paste handler hands over for identity binding. */
export type BindPastedMentionIdentities = (input: {
  /** Clipboard HTML holding the identity records — untrusted. */
  html: string;
  /** The text this paste inserted, as the mention matchers read it. */
  insertedText: string;
  /** Where the paste landed; the mention tokens inside it get fenced. */
  insertedRange: { from: number; to: number };
  view: PastedMentionOccurrenceView;
}) => void;

export type MentionPasteBinding = {
  bindPastedMentionIdentities: BindPastedMentionIdentities;
  /**
   * Record explicit user intent for a label, retiring any pending paste that
   * claimed it. Every caller is a deliberate act — a picker selection, a
   * resolved insert, a send-time persona registration.
   */
  claimMentionIntent: (label: string) => void;
  /** Drop every claim, so anything still in flight settles into nothing. */
  clearMentionIntents: () => void;
  /** Resolve once no paste verification is still deciding what to bind. */
  settlePendingMentionBindings: () => Promise<void>;
};

/**
 * How long a send waits on an in-flight paste verification.
 *
 * Generous, because the wait is bounded by a relay round trip the user cannot
 * see and the alternative is silently dropping the identity they copied. On
 * expiry the send proceeds with what the composer truthfully shows: the chip
 * never lit, so plain text sends as plain text.
 */
export const PENDING_MENTION_BINDING_TIMEOUT_MS = 10_000;

/** Same bound as the mention maps this feeds. */
const MAX_TRACKED_INTENTS = 200;

/**
 * Track a range per mention token this paste put on screen.
 *
 * Keyed by canonical label, because that is what a settlement has in hand: one
 * record can be shown twice, and either occurrence surviving means the paste
 * still shows the name. The record cap bounds distinct labels but not their
 * occurrences, so the plugin's own ceiling bounds this too — dropping the
 * tokens furthest into the paste costs an identity rather than binding one.
 */
function trackPastedMentionTokens(
  view: PastedMentionOccurrenceView,
  insertedRange: { from: number; to: number },
  records: readonly MentionIdentity[],
): Map<string, number[]> {
  const spans = findMentionTokenSpans(
    view.state.doc,
    insertedRange,
    records.map((record) => record.label),
  ).slice(0, MAX_TRACKED_OCCURRENCES);
  const tracked = new Map<string, number[]>();
  for (const span of spans) {
    const id = trackPastedMentionOccurrence(view, span.from, span.to);
    if (id === null) continue;
    const key = canonicalMentionLabel(span.label);
    const ids = tracked.get(key);
    if (ids) ids.push(id);
    else tracked.set(key, [id]);
  }
  return tracked;
}

/**
 * Whether one of this paste's tokens still reads as a mention of `label`.
 *
 * Two questions, and a settlement needs both. The tracked range has to still
 * exist — an edit that deleted or replaced the token retires it — and the
 * document at that range has to still *be* a mention, which is what catches
 * the edits a range survives by design: characters typed inside the token
 * leave both endpoints intact, and characters typed against its outer edge
 * land outside the range while destroying the word boundary a mention needs.
 * Asking `findMentionTokenSpans` for a character either side is what puts that
 * boundary back in view, since the token read alone always supplies one.
 */
function ownsLiveMentionToken(
  view: PastedMentionOccurrenceView,
  ids: readonly number[] | undefined,
  label: string,
): boolean {
  if (!ids) return false;
  return ids.some((id) => {
    const range = readPastedMentionOccurrenceRange(view, id);
    if (!range) return false;
    return findMentionTokenSpans(
      view.state.doc,
      { from: range.from - 1, to: range.to + 1 },
      [label],
    ).some((span) => span.from === range.from && span.to === range.to);
  });
}

/**
 * Bind the identities a paste is entitled to, once they check out.
 *
 * Verification can need a relay round trip, so the answer lands after the
 * insertion — which makes settlement, not the paste, the moment that has to
 * establish it is still writing what the user meant. Three fences do that,
 * one per way an unfenced settlement went wrong:
 *
 * - **Occurrence.** The `@Label` token *this* paste inserted must still be
 *   there, in the range it landed in (see `pastedMentionOccurrences`).
 *   Deleting the paste and hand-typing the same name previously bound the
 *   clipboard's pubkey to the typed text — and so did retyping the mention on
 *   its own, once the fence was the whole insertion rather than the token.
 * - **Generation.** The label's newest claim must still be this paste's. Two
 *   pastes of one label, or a picker selection made mid-flight, previously let
 *   whichever verification finished *last* own the name.
 * - **Trust.** The pair must be one this community's own state vouches for —
 *   `selectVouchedMentionIdentities`, unchanged by the fences above.
 *
 * A binding only matters at send time, and `settlePendingMentionBindings` is
 * what makes that true: the send seams await it, so a still-deciding paste
 * cannot publish a readable `@Label` with no `p` tag.
 */
export function useMentionPasteBinding({
  registerVerifiedMentionPubkey,
  verifyMentionIdentities,
}: {
  /** The non-bumping map write: settlement is not fresh user intent. */
  registerVerifiedMentionPubkey: RegisterMentionPubkey;
  verifyMentionIdentities: VerifyMentionIdentities;
}): MentionPasteBinding {
  const registerRef = React.useRef(registerVerifiedMentionPubkey);
  registerRef.current = registerVerifiedMentionPubkey;
  const verifyRef = React.useRef(verifyMentionIdentities);
  verifyRef.current = verifyMentionIdentities;
  // Newest claim per label, and the counter it is drawn from. A settlement
  // compares its own claim against the current one, so ordering decides
  // ownership rather than arrival.
  const intentsRef = React.useRef<Map<string, number>>(new Map());
  const intentCounterRef = React.useRef(0);
  const pendingRef = React.useRef<Set<Promise<void>>>(new Set());

  const claimIntent = React.useCallback((label: string): number | null => {
    const key = canonicalMentionLabel(label);
    if (!key) return null;
    const generation = ++intentCounterRef.current;
    // Re-insert so the bound below evicts by recency; an evicted claim reads
    // as "not current" at settlement, which is the fail-closed direction.
    intentsRef.current.delete(key);
    intentsRef.current.set(key, generation);
    trimMapToSize(intentsRef.current, MAX_TRACKED_INTENTS);
    return generation;
  }, []);

  const claimMentionIntent = React.useCallback(
    (label: string) => {
      claimIntent(label);
    },
    [claimIntent],
  );

  const clearMentionIntents = React.useCallback(() => {
    intentsRef.current.clear();
  }, []);

  const bindPastedMentionIdentities =
    React.useCallback<BindPastedMentionIdentities>(
      ({ html, insertedText, insertedRange, view }) => {
        const visible = selectVisibleMentionIdentities(
          parseMentionClipboardRecords(html),
          insertedText,
        );
        if (visible.length === 0) return;
        // Claimed synchronously, before anything can be awaited: a paste that
        // lands later must be able to see that it outranks this one.
        const claims = new Map<string, number | null>();
        for (const record of visible) {
          claims.set(
            canonicalMentionLabel(record.label),
            claimIntent(record.label),
          );
        }

        const tracked = trackPastedMentionTokens(view, insertedRange, visible);
        // Nothing tracked is nothing a late answer could be held to, so the
        // lookup is not worth spending: the words stay plain and tag nobody.
        if (tracked.size === 0) return;

        const settle = async () => {
          try {
            const vouched = await selectVouchedMentionIdentities(
              visible,
              verifyRef.current,
            );
            for (const record of vouched) {
              const key = canonicalMentionLabel(record.label);
              if (intentsRef.current.get(key) !== claims.get(key)) continue;
              if (!ownsLiveMentionToken(view, tracked.get(key), record.label)) {
                continue;
              }
              registerRef.current(record.label, record.pubkey, {
                isAgent: record.isAgent,
              });
            }
          } catch (error) {
            // Nothing is orphaned by giving up here — the pasted words are
            // already in the composer and simply stay plain. Retrying an
            // identity lookup the user never asked for would be the surprising
            // behaviour.
            console.warn("Could not verify pasted mention identities", error);
          } finally {
            for (const ids of tracked.values()) {
              for (const id of ids) releasePastedMentionOccurrence(view, id);
            }
          }
        };

        const settlement = settle().finally(() => {
          pendingRef.current.delete(settlement);
        });
        pendingRef.current.add(settlement);
      },
      [claimIntent],
    );

  const settlePendingMentionBindings = React.useCallback(async () => {
    const pending = pendingRef.current;
    if (pending.size === 0) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const expiry = new Promise<"expired">((resolve) => {
      timer = setTimeout(
        () => resolve("expired"),
        PENDING_MENTION_BINDING_TIMEOUT_MS,
      );
    });
    try {
      // A settlement can start another (a re-entrant paste cannot, but a
      // second composer paste racing this drain can), so loop until empty
      // rather than awaiting one snapshot of the set.
      while (pending.size > 0) {
        const outcome = await Promise.race([
          // The settlements never reject — each one catch-logs internally.
          Promise.all([...pending]).then(() => "drained" as const),
          expiry,
        ]);
        if (outcome === "expired") {
          console.warn(
            `Sending without ${pending.size} unsettled pasted mention identity check(s)`,
          );
          return;
        }
      }
    } finally {
      if (timer !== undefined) clearTimeout(timer);
    }
  }, []);

  return React.useMemo(
    () => ({
      bindPastedMentionIdentities,
      claimMentionIntent,
      clearMentionIntents,
      settlePendingMentionBindings,
    }),
    [
      bindPastedMentionIdentities,
      claimMentionIntent,
      clearMentionIntents,
      settlePendingMentionBindings,
    ],
  );
}
