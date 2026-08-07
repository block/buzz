import type { useSidebarRelayConnectionCard } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { SidebarRelayConnectionCard } from "@/features/sidebar/ui/SidebarRelayConnectionCard";
import { SidebarUpdateCard } from "@/features/settings/SidebarUpdateCard";

/**
 * The stack of transient cards above the sidebar profile row.
 *
 * Extracted from `AppSidebar.tsx` because that file sits at the 1000-line
 * ceiling enforced by `desktop/scripts/check-file-sizes.mjs`, and this is a
 * self-contained concern.
 *
 * Ordering is a priority queue, most urgent first: a broken relay connection
 * outranks an available update. Each card renders nothing when it has nothing
 * to say, so the footer does not become permanent clutter.
 *
 * Shared compute used to live here as a third card. It moved to a one-line row
 * under Agents (`SidebarMeshComputeRow`) because a persistent ~120px card is
 * the wrong price for a state that is usually "on and fine".
 */
export function SidebarFooterNotices({
  expanded,
  onDismissRelayConnectionCard,
  onDismissUpdateCard,
  onReconnectRelay,
  relayConnectionCard,
  showUpdateCard,
}: {
  /** Whether the sidebar is expanded (icon-collapse hides these cards). */
  expanded: boolean;
  onDismissRelayConnectionCard: () => void;
  onDismissUpdateCard: () => void;
  onReconnectRelay: () => void;
  relayConnectionCard: ReturnType<typeof useSidebarRelayConnectionCard>;
  showUpdateCard: boolean;
}) {
  return (
    <>
      {relayConnectionCard.showSidebarRelayConnectionCard && expanded ? (
        <SidebarRelayConnectionCard
          className="mb-2"
          isConnected={relayConnectionCard.isRelayConnectionSuccess}
          isReconnectPending={relayConnectionCard.isRelayReconnectPending}
          isWaitingOnReconnectHook={
            relayConnectionCard.isWaitingOnReconnectHook
          }
          onDismiss={onDismissRelayConnectionCard}
          onReconnect={onReconnectRelay}
        />
      ) : null}
      {showUpdateCard ? (
        <div className="mb-2 group-data-[collapsible=icon]:hidden">
          <SidebarUpdateCard onDismiss={onDismissUpdateCard} />
        </div>
      ) : null}
    </>
  );
}
