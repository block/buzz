import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  readConciergeSelection,
  SELECTION_CHANGED_EVENT,
} from "@/features/concierge/lib/conciergeSelection";
import { useIdentityQuery } from "@/shared/api/hooks";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import "./concierge.css";

/** The selected Concierge agent's display name, live across selection
 *  changes (same-tab custom event + cross-tab storage event). Falls back to
 *  "Concierge" when nothing is selected yet. */
function useConciergeName(): string {
  const selfPubkey = useIdentityQuery().data?.pubkey;
  const agentsQuery = useManagedAgentsQuery();
  const [selectedPubkey, setSelectedPubkey] = React.useState<string | null>(
    null,
  );

  React.useEffect(() => {
    if (!selfPubkey) return;
    const read = () =>
      setSelectedPubkey(
        readConciergeSelection(selfPubkey)?.agentPubkey ?? null,
      );
    read();
    window.addEventListener(SELECTION_CHANGED_EVENT, read);
    window.addEventListener("storage", read);
    return () => {
      window.removeEventListener(SELECTION_CHANGED_EVENT, read);
      window.removeEventListener("storage", read);
    };
  }, [selfPubkey]);

  const selected = agentsQuery.data?.find(
    (agent) => agent.pubkey === selectedPubkey,
  );
  return selected?.name ?? "Concierge";
}

/**
 * Composer-toolbar entry point: a compact orb button that opens the
 * Concierge, mounted next to the send arrow (mic-key placement). The
 * tooltip names the user's selected agent; the sidebar nav entry remains
 * the persistent entry point when no composer is on screen.
 */
export function ConciergeLauncher() {
  const { goConcierge } = useAppNavigation();
  const name = useConciergeName();
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          aria-label={`Open ${name}`}
          className="concierge-launcher inline-flex h-9 w-9 shrink-0 items-center justify-center rounded-full transition-colors hover:bg-muted focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          data-testid="concierge-launcher"
          onClick={() => {
            void goConcierge();
          }}
          type="button"
        >
          <span aria-hidden className="concierge-launcher__orb" />
        </button>
      </TooltipTrigger>
      <TooltipContent>{name}</TooltipContent>
    </Tooltip>
  );
}
