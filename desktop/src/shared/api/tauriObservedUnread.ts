import { invokeTauri } from "@/shared/api/tauri";
import type { ObservedUnreadEvent } from "@/features/channels/unreadChannelCounts";

export type ObservedUnreadScope = { pubkey: string; relayUrl: string };
export type ObservedUnreadProjection = {
  channelId: string;
  latest: number;
  count: number;
  badgeCount: number;
  appBadgeCount: number;
  topLevelUnread: boolean;
  highPriorityCount: number;
};
export type ObservedUnreadMembershipSeed = {
  participatedRootIds: string[];
  authoredRootIds: string[];
  mentionedRootIds: string[];
  followedRootIds: string[];
  mutedRootIds: string[];
  mutedChannelIds: string[];
};
export type ObservedUnreadWireEvent = ObservedUnreadEvent & {
  channelId: string;
};
export type ObservedUnreadResponse =
  | {
      kind: "snapshot";
      scope: ObservedUnreadScope;
      generation: string;
      revision: number;
      lastAckedSequence: number;
      migrationComplete: boolean;
      membershipSeeded: boolean;
      channels: ObservedUnreadProjection[];
    }
  | {
      kind: "delta";
      scope: ObservedUnreadScope;
      generation: string;
      baseRevision: number;
      revision: number;
      ackedSequence: number;
      upserts: ObservedUnreadProjection[];
      removed: string[];
    }
  | {
      kind: "snapshotRequired";
      scope: ObservedUnreadScope;
      generation: string;
      revision: number;
      lastAckedSequence: number;
    };

export function openObservedUnreadScope(request: {
  scope: ObservedUnreadScope;
  legacyPayload: unknown | null;
  membershipSeed: ObservedUnreadMembershipSeed | null;
}): Promise<ObservedUnreadResponse> {
  return invokeTauri("observed_unread_open_scope", { request });
}

export function ingestObservedUnread(request: {
  scope: ObservedUnreadScope;
  sequence: number;
  baseRevision: number;
  events: ObservedUnreadWireEvent[];
  channelLatest: Array<{ channelId: string; createdAt: number }>;
  markers: Array<{ contextId: string; readAt: number | null }>;
  membership: Array<{ kind: string; value: string; present: boolean }>;
  clearChannels: string[];
  clearAll: boolean;
}): Promise<ObservedUnreadResponse> {
  return invokeTauri("observed_unread_ingest", { request });
}
