import * as React from "react";

import { useComposerDockActivity } from "@/features/channels/ui/useComposerDockActivity";
import { cn } from "@/shared/lib/cn";

/**
 * Owns the composer-dock `--with-activity` class via the dock activity stores.
 * Isolates those store subscriptions from ChannelPane so timeline/composer do
 * not re-render when agents type or card-mint jobs update.
 */
export const ComposerDockFrame = React.memo(function ComposerDockFrame({
  channelId,
  children,
  typingPubkeys,
}: {
  channelId: string | null;
  children: React.ReactNode;
  typingPubkeys: readonly string[];
}) {
  const { hasActivity } = useComposerDockActivity(channelId, typingPubkeys);
  return (
    <div
      className={cn(
        "composer-dock composer-overlay-corner-masks relative pointer-events-auto",
        hasActivity && "composer-dock--with-activity",
      )}
    >
      {children}
    </div>
  );
});
