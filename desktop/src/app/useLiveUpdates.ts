import { useCommunityEmojiLiveUpdates } from "@/features/custom-emoji/hooks";
import { useStickerLiveUpdates } from "@/features/stickers/hooks";

/**
 * Subscribes the app shell to all workspace-scoped live-update streams
 * (community custom emoji, sticker catalog). Adding a new stream here keeps
 * AppShell's hook list flat.
 */
export function useLiveUpdates(): void {
  useCommunityEmojiLiveUpdates();
  useStickerLiveUpdates();
}
