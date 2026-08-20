import * as React from "react";

import {
  fibreSeenStorageKey,
  readFibreSeenMap,
  writeFibreSeenMap,
  type FibreSeenMap,
} from "@/features/home/ui/fibre/fibreSeen";
import type { Fibre } from "@/features/triage/api";

export function useFibreSeenState(
  pubkey: string | undefined,
  relayUrl: string | undefined,
) {
  const key = fibreSeenStorageKey(relayUrl, pubkey);
  const [seenAtById, setSeenAtById] = React.useState<FibreSeenMap>(() =>
    readFibreSeenMap(key),
  );

  React.useEffect(() => {
    setSeenAtById(readFibreSeenMap(key));
  }, [key]);

  const markSeen = React.useCallback(
    (fibre: Fibre) => {
      setSeenAtById((current) => {
        if (current[fibre.id] === fibre.updatedAt) return current;
        const next = { ...current, [fibre.id]: fibre.updatedAt };
        writeFibreSeenMap(key, next);
        return next;
      });
    },
    [key],
  );

  return { markSeen, seenAtById };
}
