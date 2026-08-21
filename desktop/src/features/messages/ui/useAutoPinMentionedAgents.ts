import * as React from "react";
import { toast } from "sonner";

import {
  getPersistentAgentAudienceSnapshot,
  removePersistentAgentAudienceMember,
} from "@/features/messages/lib/persistentAgentAudience";
import { normalizePubkey } from "@/shared/lib/pubkey";

type Options = {
  audienceScope: string | null;
  enabled: boolean;
  addPubkey: (pubkey: string) => void;
  getDisplayName: (pubkey: string) => string | null | undefined;
  onOpenOptions: () => void;
  onPulse: (pubkey: string) => void;
};

export function useAutoPinMentionedAgents({
  audienceScope,
  enabled,
  addPubkey,
  getDisplayName,
  onOpenOptions,
  onPulse,
}: Options) {
  return React.useCallback(
    (pubkeys: readonly string[]) => {
      if (!audienceScope || !enabled) return;
      const pinnedPubkeys = new Set(
        getPersistentAgentAudienceSnapshot().audiences[audienceScope] ?? [],
      );
      const newlyPinnedPubkeys: string[] = [];
      for (const pubkey of pubkeys) {
        const normalizedPubkey = normalizePubkey(pubkey);
        if (!pinnedPubkeys.has(normalizedPubkey)) {
          pinnedPubkeys.add(normalizedPubkey);
          newlyPinnedPubkeys.push(normalizedPubkey);
        }
        addPubkey(normalizedPubkey);
        onPulse(normalizedPubkey);
      }
      if (newlyPinnedPubkeys.length === 0) return;

      const showToast = (title: string) => {
        toast.success(title, {
          action: {
            label: "Undo",
            onClick: () => {
              for (const pubkey of newlyPinnedPubkeys) {
                removePersistentAgentAudienceMember(audienceScope, pubkey);
              }
              onOpenOptions();
            },
          },
        });
      };
      if (newlyPinnedPubkeys.length === 1) {
        const displayName = getDisplayName(newlyPinnedPubkeys[0])?.trim();
        showToast(
          displayName
            ? `${displayName} will be mentioned automatically`
            : "Agent will be mentioned automatically",
        );
      } else {
        showToast(
          `${newlyPinnedPubkeys.length} agents will be mentioned automatically`,
        );
      }
    },
    [addPubkey, audienceScope, enabled, getDisplayName, onOpenOptions, onPulse],
  );
}
