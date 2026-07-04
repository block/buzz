import { AnimatePresence, motion } from "motion/react";

import { SidebarRelayConnectionCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { cn } from "@/shared/lib/cn";
import { useSidebar } from "@/shared/ui/sidebar";

type RelayConnectionOverlayProps = {
  errorMessage?: string;
  hasWorkspaceRail?: boolean;
  isHuddleDrawerOpen?: boolean;
  relayUrl?: string | null;
};

/**
 * Fixed bottom-left overlay that shows the relay reconnect card when the
 * sidebar is collapsed. When the sidebar is open, the card lives in the
 * sidebar footer instead (and this overlay is hidden). Offsets itself for
 * the workspace rail (48px) and huddle drawer when present.
 */
export function RelayConnectionOverlay({
  errorMessage,
  hasWorkspaceRail,
  isHuddleDrawerOpen,
  relayUrl,
}: RelayConnectionOverlayProps) {
  const card = useSidebarRelayConnectionCard(errorMessage, relayUrl);
  const { open: sidebarOpen } = useSidebar();

  const shouldShow = card.showSidebarRelayConnectionCard && !sidebarOpen;

  return (
    <AnimatePresence>
      {shouldShow ? (
        <motion.div
          animate={{ opacity: 1, y: 0 }}
          className={cn(
            "pointer-events-none fixed z-50 w-[284px]",
            hasWorkspaceRail ? "left-[60px]" : "left-3",
            isHuddleDrawerOpen
              ? "bottom-[calc(var(--buzz-huddle-drawer-height,0px)+12px)]"
              : "bottom-3",
          )}
          exit={{ opacity: 0, y: 20 }}
          initial={{ opacity: 0, y: -20 }}
          key="relay-connection-overlay"
          transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
        >
          <div className="pointer-events-auto rounded-xl bg-background shadow-md">
            <SidebarRelayConnectionCard
              isConnected={card.isRelayConnectionSuccess}
              isReconnectPending={card.isRelayReconnectPending}
              isWaitingOnReconnectHook={card.isWaitingOnReconnectHook}
              onDismiss={card.onDismissRelayConnectionCard}
              onReconnect={card.onReconnectRelay}
            />
          </div>
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
