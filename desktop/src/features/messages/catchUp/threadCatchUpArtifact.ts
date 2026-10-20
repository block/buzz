import type {
  CatchUpPoint,
  SharedCatchUp,
  SharedCatchUpList,
} from "@/shared/api/tauriThreadCatchUp";

/**
 * Read-side projection of a shared recap. The recap text never changes;
 * corrections layer on top in signed `prev`-chain order as native returns
 * them, so the newest correction for a point is its current reading and the
 * generated text stays visible as the first history entry.
 */

export type CatchUpPointRevision = {
  key: string;
  kind: "generated" | "correction";
  text: string;
  /** Signer of the revision; null for generated text, which has no author. */
  authorPubkey: string | null;
  /** Null when the sharing revision is outside the retrieved history. */
  createdAt: number | null;
};

export type SharedCatchUpPointView = {
  point: CatchUpPoint;
  currentText: string;
  corrected: boolean;
  /** Oldest first: the generated text, then each correction to this point. */
  history: CatchUpPointRevision[];
};

export function projectSharedCatchUpPoints(
  shared: SharedCatchUp,
): SharedCatchUpPointView[] {
  return shared.recap.points.map((point) => {
    const history: CatchUpPointRevision[] = [
      {
        key: `${shared.artifactId}:${point.id}:generated`,
        kind: "generated",
        text: point.text,
        authorPubkey: null,
        createdAt: shared.historyComplete ? shared.createdAt : null,
      },
    ];
    for (const correction of shared.corrections) {
      if (correction.pointId !== point.id) continue;
      history.push({
        key: correction.eventId,
        kind: "correction",
        text: correction.text,
        authorPubkey: correction.authorPubkey,
        createdAt: correction.createdAt,
      });
    }
    const latest = history[history.length - 1];
    return {
      point,
      currentText: latest.text,
      corrected: latest.kind === "correction",
      history,
    };
  });
}

/** Replace a recap by identity, keeping the list most recently changed first. */
export function upsertSharedCatchUp(
  list: SharedCatchUpList | undefined,
  shared: SharedCatchUp,
): SharedCatchUpList {
  const others = (list?.recaps ?? []).filter(
    (entry) => entry.artifactId !== shared.artifactId,
  );
  return {
    recaps: [shared, ...others],
    unreadableCount: list?.unreadableCount ?? 0,
    complete: list?.complete ?? true,
  };
}

/** The time a shared recap last changed: its newest correction or creation. */
export function sharedCatchUpUpdatedAt(shared: SharedCatchUp): number {
  return shared.corrections.reduce(
    (latest, correction) => Math.max(latest, correction.createdAt),
    shared.createdAt,
  );
}

export type SharedCatchUpAttribution =
  | { kind: "sharer"; pubkey: string; createdAt: number }
  | { kind: "incomplete" };

/**
 * Who shared the recap. With incomplete history native reports the earliest
 * retrieved revision, which may be a corrector rather than the sharer, so the
 * sharer is shown as unknown instead of naming the wrong person.
 */
export function sharedCatchUpAttribution(
  shared: SharedCatchUp,
): SharedCatchUpAttribution {
  if (!shared.historyComplete) return { kind: "incomplete" };
  return {
    kind: "sharer",
    pubkey: shared.authorPubkey,
    createdAt: shared.createdAt,
  };
}
