import * as React from "react";

import { canonicalRelayUrl } from "@/features/agents/managedAgentRuntimeStatus";
import { useOpenDmMutation } from "@/features/channels/hooks";
import type { OpenDmInput } from "@/shared/api/tauriChannels";

type OpenDmScope = {
  relayUrl?: string;
  signerPubkey?: string;
};

export function useScopedOpenDmNavigation({
  goChannel,
  relayUrl,
  signerPubkey,
}: OpenDmScope & {
  goChannel: (channelId: string) => Promise<unknown>;
}) {
  const openDmMutation = useOpenDmMutation();
  const scopeRef = React.useRef<OpenDmScope>({});
  scopeRef.current = { relayUrl, signerPubkey };

  return React.useCallback(
    async (input: OpenDmInput) => {
      const directMessage = await openDmMutation.mutateAsync(input);
      const currentScope = scopeRef.current;
      if (
        input.expectedRelayUrl &&
        canonicalRelayUrl(input.expectedRelayUrl) !==
          canonicalRelayUrl(currentScope.relayUrl ?? "")
      ) {
        return;
      }
      if (
        input.expectedSignerPubkey &&
        input.expectedSignerPubkey.toLowerCase() !==
          currentScope.signerPubkey?.toLowerCase()
      ) {
        return;
      }
      await goChannel(directMessage.id);
    },
    [goChannel, openDmMutation],
  );
}
