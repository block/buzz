import type { ConnectionState } from "@/shared/api/relayClientShared";

const STORAGE_KEY_PREFIX = "buzz-thread-reply-desktop-seen.v1";
export const THREAD_REPLY_SEEN_MAX_ITEMS = 2_000;

type SeenThreadReply = {
  eventId: string;
  channelId: string;
  createdAt: number;
};

type SeenStorage = Pick<Storage, "getItem" | "setItem">;

function storageKey(pubkey: string) {
  return `${STORAGE_KEY_PREFIX}:${pubkey}`;
}

function defaultStorage(): SeenStorage | undefined {
  if (typeof window === "undefined") {
    return undefined;
  }

  try {
    return window.localStorage;
  } catch {
    return undefined;
  }
}

function isSeenThreadReply(value: unknown): value is SeenThreadReply {
  if (typeof value !== "object" || value === null) {
    return false;
  }

  const candidate = value as Partial<SeenThreadReply>;
  return (
    typeof candidate.eventId === "string" &&
    typeof candidate.channelId === "string" &&
    typeof candidate.createdAt === "number" &&
    Number.isFinite(candidate.createdAt)
  );
}

function readSeenThreadReplies(
  pubkey: string,
  storage: SeenStorage | undefined,
): SeenThreadReply[] {
  if (!storage || pubkey.length === 0) {
    return [];
  }

  try {
    const rawValue = storage.getItem(storageKey(pubkey));
    if (!rawValue) {
      return [];
    }

    const parsed: unknown = JSON.parse(rawValue);
    return Array.isArray(parsed) ? parsed.filter(isSeenThreadReply) : [];
  } catch {
    return [];
  }
}

/**
 * Remembers thread replies already offered to the desktop notification path.
 *
 * Reconnect history is delivered newest-page first, so cap eviction is
 * suspended during socket reconnect and retryable CLOSED recovery; otherwise
 * a burst could discard an older boundary ID before its replay page arrives.
 * Age eviction follows each broad subscription's original `since`, because
 * that is the actual history window restored by both recovery paths.
 */
export class ThreadReplyNotificationDedupe {
  private readonly pubkey: string;
  private readonly storage: SeenStorage | undefined;
  private readonly seenById = new Map<string, SeenThreadReply>();
  private readonly channelReplayFloors = new Map<string, number>();
  private activeChannelIds: Set<string> | undefined;
  private connectionReplayInProgress = false;
  private subscriptionReplayCount = 0;
  private persistenceDirty = false;
  private persistenceDisabled = false;

  constructor(
    pubkey: string,
    storage: SeenStorage | undefined = defaultStorage(),
  ) {
    this.pubkey = pubkey;
    this.storage = storage;
    for (const record of readSeenThreadReplies(pubkey, storage)) {
      this.seenById.set(record.eventId, record);
    }
    // Do not cap persisted data until active channels are known. Departed
    // records must be removed before they can displace an active-channel ID.
  }

  /** Removes replay state for channels without an active broad subscription. */
  reconcileActiveChannels(channelIds: Iterable<string>) {
    const activeChannelIds = new Set(channelIds);
    for (const channelId of this.channelReplayFloors.keys()) {
      if (!activeChannelIds.has(channelId)) {
        this.channelReplayFloors.delete(channelId);
      }
    }
    for (const [eventId, record] of this.seenById) {
      if (!activeChannelIds.has(record.channelId)) {
        this.seenById.delete(eventId);
      }
    }

    this.activeChannelIds = activeChannelIds;
    this.pruneAllChannels();
    if (!this.isReplayInProgress()) {
      this.enforceSizeCap();
    }
    this.persist();
  }

  setChannelReplayFloor(channelId: string, replayFloor: number) {
    this.activeChannelIds?.add(channelId);
    this.channelReplayFloors.set(channelId, replayFloor);
    if (this.activeChannelIds && !this.isReplayInProgress()) {
      this.pruneChannel(channelId, replayFloor);
      this.enforceSizeCap();
      this.persist();
    }
  }

  /** Records an observed reply and returns true only for its first sighting. */
  record(channelId: string, eventId: string, createdAt: number) {
    if (this.activeChannelIds && !this.activeChannelIds.has(channelId)) {
      return false;
    }
    if (this.seenById.has(eventId)) {
      return false;
    }

    this.seenById.set(eventId, { eventId, channelId, createdAt });
    if (this.activeChannelIds && !this.isReplayInProgress()) {
      this.enforceSizeCap();
    }
    this.persist();
    return true;
  }

  handleConnectionState(state: ConnectionState) {
    if (state === "reconnecting" || state === "stalled") {
      if (!this.connectionReplayInProgress) {
        const replayWasInProgress = this.isReplayInProgress();
        this.connectionReplayInProgress = true;
        if (replayWasInProgress) return;
        // The original long-lived live REQ is restored on reconnect, so its
        // immutable `since` is the actual replay floor for persisted IDs.
        this.beginReplay();
      }
      return;
    }

    if (state === "connected" && this.connectionReplayInProgress) {
      this.connectionReplayInProgress = false;
      this.finishReplayIfIdle();
    }
  }

  handleSubscriptionRecoveryState(recovering: boolean) {
    if (recovering) {
      const replayWasInProgress = this.isReplayInProgress();
      this.subscriptionReplayCount += 1;
      if (!replayWasInProgress) {
        this.beginReplay();
      }
      return;
    }

    if (this.subscriptionReplayCount === 0) return;
    this.subscriptionReplayCount -= 1;
    this.finishReplayIfIdle();
  }

  /** Flushes pending replay state when this dedupe instance is handed off. */
  flush() {
    this.pruneAllChannels();
    if (this.activeChannelIds) {
      this.enforceSizeCap();
    }
    this.flushPersistence();
  }

  private pruneAllChannels() {
    for (const [channelId, replayFloor] of this.channelReplayFloors) {
      this.pruneChannel(channelId, replayFloor);
    }
  }

  private pruneChannel(channelId: string, replayFloor: number) {
    for (const [eventId, record] of this.seenById) {
      if (record.channelId === channelId && record.createdAt < replayFloor) {
        this.seenById.delete(eventId);
      }
    }
  }

  private enforceSizeCap() {
    const excess = this.seenById.size - THREAD_REPLY_SEEN_MAX_ITEMS;
    if (excess <= 0) {
      return;
    }

    const oldest = [...this.seenById.values()]
      .sort((left, right) => left.createdAt - right.createdAt)
      .slice(0, excess);
    for (const record of oldest) {
      this.seenById.delete(record.eventId);
    }
  }

  private persist() {
    this.persistenceDirty = true;
    if (this.isReplayInProgress()) {
      return;
    }
    this.flushPersistence();
  }

  private isReplayInProgress() {
    return this.connectionReplayInProgress || this.subscriptionReplayCount > 0;
  }

  private beginReplay() {
    this.pruneAllChannels();
    this.persist();
  }

  private finishReplayIfIdle() {
    if (this.isReplayInProgress()) return;
    this.pruneAllChannels();
    if (this.activeChannelIds) {
      this.enforceSizeCap();
    }
    this.persist();
  }

  private flushPersistence() {
    if (!this.persistenceDirty) {
      return;
    }
    if (!this.storage || this.pubkey.length === 0 || this.persistenceDisabled) {
      this.persistenceDirty = false;
      return;
    }

    try {
      this.storage.setItem(
        storageKey(this.pubkey),
        JSON.stringify([...this.seenById.values()]),
      );
      this.persistenceDirty = false;
    } catch {
      // In-memory dedupe still works when localStorage is unavailable/full.
      // Disable more serialization attempts for this instance so a full store
      // cannot add repeated main-thread work after the first failed flush.
      this.persistenceDisabled = true;
      this.persistenceDirty = false;
    }
  }
}

export function shouldDeliverThreadReplyDesktopNotification({
  isFirstSeen,
  isEligible,
  isActiveChannel,
  notifyForActiveChannel,
}: {
  isFirstSeen: boolean;
  isEligible: boolean;
  isActiveChannel: boolean;
  notifyForActiveChannel: boolean;
}) {
  return (
    isFirstSeen && isEligible && (!isActiveChannel || notifyForActiveChannel)
  );
}
