import { useOffscreenActivityChannelIds } from "@/features/sidebar/lib/useOffscreenActivityChannelIds";
import { useUnreadOverflow } from "@/features/sidebar/lib/useUnreadOverflow";

type ActivityOptions = Parameters<typeof useOffscreenActivityChannelIds>[0];
type ScrollRef = Parameters<typeof useUnreadOverflow>[0]["scrollRef"];

export function useSidebarActivityOverflow({
  scrollRef,
  ...activityOptions
}: ActivityOptions & { scrollRef: ScrollRef }) {
  return useUnreadOverflow({
    scrollRef,
    unreadChannelIds: useOffscreenActivityChannelIds(activityOptions),
  });
}
