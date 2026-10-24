// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { useUiLanguage } from "@/shared/i18n/uiTranslation";
import * as React from "react";

import { useCommunities } from "@/features/communities/useCommunities";
import type { Channel } from "@/shared/api/types";

const MembersSidebar = React.lazy(async () => {
  const module = await import("@/features/channels/ui/MembersSidebar");
  return { default: module.MembersSidebar };
});

export function HomeMembersSidebarOverlay({
  channel,
  currentPubkey,
  onClose,
}: {
  channel: Channel | null;
  currentPubkey?: string;
  onClose: () => void;
}) {
  useUiLanguage();

  const { activeCommunity } = useCommunities();

  if (!channel) return null;

  return (
    <React.Suspense fallback={null}>
      <MembersSidebar
        channel={channel}
        currentPubkey={currentPubkey}
        onOpenChange={(nextOpen) => {
          if (!nextOpen) onClose();
        }}
        open={true}
        relayUrl={activeCommunity?.relayUrl}
      />
    </React.Suspense>
  );
}
