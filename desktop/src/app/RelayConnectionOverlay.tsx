import { AnimatePresence, motion } from "motion/react";
import { AlertCircle } from "lucide-react";

import { SidebarRelayConnectionCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import type { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { cn } from "@/shared/lib/cn";
import { useIsMobile } from "@/shared/hooks/use-mobile";
import { useRelayConnection } from "@/shared/api/useRelayConnection";
import { useSidebar } from "@/shared/ui/sidebar";

type RelayConnectionOverlayProps = {
  card: ReturnType<typeof useSidebarRelayConnectionCard>;
  errorMessage?: string;
  hasWorkspaceRail?: boolean;
  isHuddleDrawerOpen?: boolean;
};

/**
 * Fixed bottom-left overlay that shows the relay reconnect card when the
 * sidebar is collapsed. When the sidebar is open, the card lives in the
 * sidebar footer instead (and this overlay is hidden). Offsets itself for
 * the workspace rail (48px) and huddle drawer when present.
 *
 * Also surfaces non-unreachable disconnect errors (e.g. auth rejections)
 * when the sidebar is hidden, since those errors are only rendered inside
 * the sidebar content area which is off-canvas when collapsed.
 */
export function RelayConnectionOverlay({
  card,
  errorMessage,
  hasWorkspaceRail,
  isHuddleDrawerOpen,
}: RelayConnectionOverlayProps) {
  const { open: sidebarOpen, openMobile } = useSidebar();
  const isMobile = useIsMobile();
  const connectionState = useRelayConnection();

  // Show the overlay when the sidebar surface isn't visible:
  // - Desktop: sidebar is collapsed (open === false)
  // - Mobile: the sheet is closed (openMobile === false)
  const isSidebarSurfaceHidden = isMobile ? !openMobile : !sidebarOpen;
  const shouldShowReconnectCard =
    card.showSidebarRelayConnectionCard && isSidebarSurfaceHidden;

  // Show a non-unreachable error (e.g. auth rejection) when the sidebar is
  // hidden and the reconnect card isn't already covering it.
  const hasNonUnreachableError =
    Boolean(errorMessage) &&
    !card.hasRelayUnreachableError &&
    connectionState === "disconnected";
  const shouldShowErrorFallback =
    hasNonUnreachableError &&
    isSidebarSurfaceHidden &&
    !shouldShowReconnectCard;

  return (
    <AnimatePresence>
      {shouldShowReconnectCard ? (
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
      {shouldShowErrorFallback ? (
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
          key="relay-error-overlay"
          transition={{ duration: 0.25, ease: [0.22, 1, 0.36, 1] }}
        >
          <div
            className="pointer-events-auto flex items-center gap-2 rounded-xl bg-background px-3 py-2.5 text-sm text-destructive shadow-md"
            data-testid="relay-error-overlay"
            role="alert"
          >
            <AlertCircle aria-hidden="true" className="h-4 w-4 shrink-0" />
            <span className="flex-1">{errorMessage}</span>
          </div>
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
