import * as React from "react";
import { toast } from "sonner";

import {
  promotePersistentAgentAudienceIfUnchanged,
  removePersistentAgentAudienceMembersIfUnchanged,
} from "@/features/messages/lib/persistentAgentAudience";
import { normalizePubkey } from "@/shared/lib/pubkey";

type Options = {
  audienceScope: string | null;
  enabled: boolean;
  getDisplayName: (pubkey: string) => string | null | undefined;
  onOpenOptions: () => void;
  onPulse: (pubkey: string) => void;
};

export function useAutoPinMentionedAgents({
  audienceScope,
  enabled,
  getDisplayName,
  onOpenOptions,
  onPulse,
}: Options) {
  return React.useCallback(
    ({
      expectedRevision,
      pubkeys,
    }: {
      expectedRevision: number;
      pubkeys: readonly string[];
    }) => {
      if (!audienceScope || !enabled) return;
      const normalizedPubkeys = [
        ...new Set(pubkeys.map(normalizePubkey)),
      ].filter(Boolean);
      const appliedRevision = promotePersistentAgentAudienceIfUnchanged({
        expectedRevision,
        pubkeys: normalizedPubkeys,
        scope: audienceScope,
      });
      if (appliedRevision === null) return;
      for (const pubkey of normalizedPubkeys) onPulse(pubkey);

      const displayName =
        normalizedPubkeys.length === 1
          ? getDisplayName(normalizedPubkeys[0])?.trim()
          : null;
      const title = displayName
        ? `${displayName} will be mentioned automatically`
        : normalizedPubkeys.length === 1
          ? "Agent will be mentioned automatically"
          : `${normalizedPubkeys.length} agents will be mentioned automatically`;
      toast.success(title, {
        action: {
          label: "Undo",
          onClick: () => {
            if (
              removePersistentAgentAudienceMembersIfUnchanged({
                expectedRevision: appliedRevision,
                pubkeys: normalizedPubkeys,
                scope: audienceScope,
              })
            ) {
              onOpenOptions();
            }
          },
        },
      });
    },
    [audienceScope, enabled, getDisplayName, onOpenOptions, onPulse],
  );
}
