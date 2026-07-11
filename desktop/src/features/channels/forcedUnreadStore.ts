import { makeRootIdStore } from "@/features/channels/unreadRootIdStore";

/**
 * Per-pubkey localStorage store for channels manually marked unread via
 * right-click → "mark unread". Keyed by channelId (not thread-root). Persisted
 * so the sidebar badge survives reload and the rail observer can read it for
 * inactive workspaces.
 *
 * Cleared on: channel-open, identity change, and when a cross-device synced
 * read-marker covers the channel (drainSyncedAdvances path in useUnreadChannels).
 *
 * NOT synced to the relay — NIP-RS markers are monotonic and cannot represent
 * a retrograde "unread" state. localStorage is best-effort (per-device).
 */
export const forcedUnreadStore = makeRootIdStore("buzz-forced-unread.v1");
