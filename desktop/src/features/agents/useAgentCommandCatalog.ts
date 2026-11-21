import * as React from "react";

import {
  getAgentCommandCatalog,
  subscribeAgentCommandCatalog,
} from "./agentCommandCatalog";

/** Subscribe to the current owner's command catalog without copying snapshots. */
export function useAgentCommandCatalog(ownerPubkey: string | null) {
  return React.useSyncExternalStore(
    subscribeAgentCommandCatalog,
    () => getAgentCommandCatalog(ownerPubkey),
    () => getAgentCommandCatalog(null),
  );
}
