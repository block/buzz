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
      const promotion = promotePersistentAgentAudienceIfUnchanged({
        expectedRevision,
        pubkeys: normalizedPubkeys,
        scope: audienceScope,
      });
      if (promotion === null) return;
      const { promotedPubkeys, revision } = promotion;
      for (const pubkey of promotedPubkeys) onPulse(pubkey);

      const displayName =
        promotedPubkeys.length === 1
          ? getDisplayName(promotedPubkeys[0])?.trim()
          : null;
      const title = displayName
        ? `${displayName} will be mentioned automatically`
        : promotedPubkeys.length === 1
          ? "Agent will be mentioned automatically"
          : `${promotedPubkeys.length} agents will be mentioned automatically`;
      toast.success(title, {
        action: {
          label: "Undo",
          onClick: () => {
            if (
              removePersistentAgentAudienceMembersIfUnchanged({
                expectedRevision: revision,
                pubkeys: promotedPubkeys,
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
