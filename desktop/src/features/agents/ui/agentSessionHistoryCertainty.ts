/**
 * Whether an empty transcript is a fact or merely an absence of evidence.
 *
 * Archived observer history is only readable when the viewing identity holds an
 * `owner_p` save subscription and the backfill index has been populated for the
 * channel. When either is missing, `useLoadArchivedObserverEvents` returns no
 * rows and forces `hasOlderArchived` to false — the same shape as a channel
 * where the agent genuinely never ran. Rendering "No ACP activity yet" in that
 * state asserts something we did not check, and a supervisor who reads it may
 * conclude an agent did nothing when its history was simply unavailable.
 *
 * So the distinction is preserved here rather than collapsed at the call site.
 * The uncertain variants name WHICH check came up short, because that is what
 * the reader needs in order to know whether to wait, look elsewhere, or trust
 * what they see.
 */
export type TranscriptHistoryCertainty =
  | "known-empty"
  | UncertainHistoryCertainty;

/** Specific reason the history could not be established as complete. */
export type UncertainHistoryCertainty =
  | "unknown-subscription-unresolved"
  | "unknown-not-indexed"
  | "unknown-archive-loading";

export type TranscriptHistoryInputs = {
  /**
   * Result of the `owner_p` save-subscription check: null while unresolved,
   * false when the identity has no subscription (or the lookup failed).
   */
  hasSubscription: boolean | null;
  /** Channel whose archive would be read; null means no archive was consulted. */
  channelId: string | null;
  /** True once the archive backfill/hydration pass finished for this channel. */
  archiveHydrated: boolean;
};

/**
 * Decide whether emptiness can be stated as fact.
 *
 * Only an identity with a resolved subscription, a real channel, and a
 * completed hydration pass has actually looked at the whole history. Every
 * other combination is uncertainty and must say so.
 */
export function getTranscriptHistoryCertainty({
  hasSubscription,
  channelId,
  archiveHydrated,
}: TranscriptHistoryInputs): TranscriptHistoryCertainty {
  if (hasSubscription === null) return "unknown-subscription-unresolved";
  if (hasSubscription === false) return "unknown-not-indexed";
  // A subscription exists but no channel was consulted: nothing was read from
  // the archive, which is indistinguishable from an unfinished read.
  if (!channelId) return "unknown-archive-loading";
  if (!archiveHydrated) return "unknown-archive-loading";
  return "known-empty";
}

/** True when an empty transcript must not be presented as "no activity". */
export function isUncertainHistory(
  certainty: TranscriptHistoryCertainty,
): certainty is UncertainHistoryCertainty {
  return certainty !== "known-empty";
}

export type TranscriptEmptyCopy = {
  title: string;
  description: string;
};

/**
 * Copy for the uncertain case.
 *
 * Phrased as a statement about what this view can see rather than about what
 * the agent did, and it never uses "no activity" — the whole point is that we
 * do not know. Each variant names the specific gap so the message stays
 * actionable instead of vaguely ominous.
 *
 * This is the only place uncertain-history wording lives. Rendering code must
 * call it rather than inlining its own, so the two cannot drift apart.
 */
export function getUncertainHistoryCopy(
  certainty: UncertainHistoryCertainty,
): TranscriptEmptyCopy {
  if (certainty === "unknown-subscription-unresolved") {
    return {
      title: "Checking for earlier activity",
      description: "Looking up whether archived history is available here.",
    };
  }
  if (certainty === "unknown-not-indexed") {
    return {
      title: "Earlier activity may not be shown",
      description:
        "Archived history isn't indexed for this identity, so only activity observed live appears here.",
    };
  }
  return {
    title: "Earlier activity may still be loading",
    description:
      "Archived history for this channel hasn't finished loading, so older activity may be missing.",
  };
}
