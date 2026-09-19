import * as React from "react";

import type { TranscriptDisplayBlock } from "./agentSessionTranscriptGrouping";
import {
  resolveTranscriptToolRunGroupKeys,
  scopeToolRunGroupKey,
} from "./agentSessionToolRunGroupKey";
import {
  readToolRunGroupIdentityTable,
  writeToolRunGroupIdentityTable,
} from "./agentSessionToolRunViewStore";

const TranscriptToolRunGroupKeyContext = React.createContext<
  ReadonlyMap<string, string>
>(new Map());

export const TranscriptToolRunGroupKeyProvider =
  TranscriptToolRunGroupKeyContext.Provider;

/**
 * Resolve one durable identity per rendered tool-run group for this transcript.
 *
 * Runs at the list level rather than inside each group card because identity is
 * allocated across the whole pass: a group claims the identity of a leaf it
 * kept, and no two groups may claim the same one. A card resolving on its own
 * could not see its siblings' claims.
 *
 * The recognition table is written during render (not in an effect) so the very
 * first paint after a leaf is ejected already uses the carried-over identity —
 * an effect would let the card mount once at policy default and visibly reset a
 * reader's collapse before correcting itself.
 */
export function useResolvedToolRunGroupKeys(
  scope: string,
  blocks: readonly TranscriptDisplayBlock[],
): ReadonlyMap<string, string> {
  return React.useMemo(() => {
    const { keysBySummaryId, table } = resolveTranscriptToolRunGroupKeys(
      blocks,
      readToolRunGroupIdentityTable(scope),
    );
    writeToolRunGroupIdentityTable(scope, table);
    return new Map(
      [...keysBySummaryId].map(([summaryId, groupKey]) => [
        summaryId,
        scopeToolRunGroupKey(scope, groupKey),
      ]),
    );
  }, [blocks, scope]);
}

/**
 * The durable identity for one group card, falling back to the summary's own id
 * if this transcript never resolved it (e.g. a card rendered outside a
 * provider). The fallback is per-summary, so it is never shared between groups.
 */
export function useToolRunGroupKey(summaryId: string, anchorKey: string) {
  const keys = React.useContext(TranscriptToolRunGroupKeyContext);
  return keys.get(summaryId) ?? anchorKey;
}
