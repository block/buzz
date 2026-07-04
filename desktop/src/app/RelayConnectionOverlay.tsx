import { AnimatePresence, motion } from "motion/react";

import { SidebarRelayConnectionCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { useSidebar } from "@/shared/ui/sidebar";

type RelayConnectionOverlayProps = {
  errorMessage?: string;
  relayUrl?: string | null;
};

/**
 * Fixed bottom-left overlay that shows the relay reconnect card when the
 * sidebar is collapsed. When the sidebar is open, the card lives in the
 * sidebar footer instead. On sidebar close, the card slides down into this
 * fixed position with a motion animation.
 */
export function RelayConnectionOverlay({
  errorMessage,
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
          className="pointer-events-none fixed bottom-3 left-3 z-50 w-[284px]"
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
