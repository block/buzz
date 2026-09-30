import { relayClient } from "@/shared/api/relayClient";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_COMMUNITY_THEME } from "@/shared/constants/kinds";
import {
  parseCommunityThemePreference,
  sameCommunityThemePreference,
  type CommunityThemePreference,
} from "./communityThemePreference";

const D_TAG = "community-theme";
const DEBOUNCE_MS = 2_000;
const PUBLISH_RETRY_BASE_MS = 1_000;
const PUBLISH_RETRY_MAX_MS = 30_000;

export type PublishedCommunityTheme = {
  preference: CommunityThemePreference;
  createdAt: number;
  eventId: string;
};

export type RemoteCommunityTheme = {
  preference: CommunityThemePreference;
  createdAt: number;
  eventId: string;
};

export type RemoteCommunityThemeResult =
  | { status: "valid"; remote: RemoteCommunityTheme }
  | { status: "absent" | "invalid" | "unavailable" };

export function isNewerCommunityThemeCoordinate(
  candidate: { createdAt: number; eventId: string },
  current: { createdAt: number; eventId: string },
): boolean {
  return (
    candidate.createdAt > current.createdAt ||
    (candidate.createdAt === current.createdAt &&
      (current.eventId === "" || candidate.eventId < current.eventId))
  );
}

export type CommunityThemeHydrationResult =
  | { status: "valid"; remote: RemoteCommunityTheme }
  | { status: "confirmed-absent" }
  | { status: "invalid" | "unavailable" };

export function communityThemeHydrationRemote(
  result: CommunityThemeHydrationResult,
): RemoteCommunityTheme | null {
  return result.status === "valid" ? result.remote : null;
}

export function shouldSeedCommunityTheme(
  result: CommunityThemeHydrationResult,
): boolean {
  return result.status === "confirmed-absent";
}

async function decryptAndParse(
  event: RelayEvent,
): Promise<RemoteCommunityTheme | null> {
  try {
    const plaintext = await nip44DecryptFromSelf(event.content);
    const preference = parseCommunityThemePreference(JSON.parse(plaintext));
    return preference
      ? { preference, createdAt: event.created_at, eventId: event.id }
      : null;
  } catch {
    return null;
  }
}

export class CommunityThemeSyncManager {
  private readonly pubkey: string;
  private debounceTimer: number | null = null;
  private destroyed = false;
  private lastRemoteCreatedAt = 0;
  private lastRemoteEventId = "";
  private latestRemote: RemoteCommunityTheme | null = null;
  private observedInvalidLiveRemote = false;
  private lastPublished: PublishedCommunityTheme | null = null;
  private pending: CommunityThemePreference | null = null;
  private publishInFlight = false;
  private publishRetryAttempt = 0;
  private readonly remoteProcessing = new Set<Promise<unknown>>();
  private readonly onPublished: (published: PublishedCommunityTheme) => void;

  constructor(
    pubkey: string,
    onPublished: (published: PublishedCommunityTheme) => void = () => {},
  ) {
    this.pubkey = pubkey;
    this.onPublished = onPublished;
  }

  async fetchRemote(): Promise<RemoteCommunityThemeResult> {
    try {
      const events = await relayClient.fetchEvents({
        kinds: [KIND_COMMUNITY_THEME],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 1,
      });
      if (events.length === 0) return { status: "absent" };
      if (events[0].pubkey !== this.pubkey) return { status: "invalid" };
      const processing = decryptAndParse(events[0]);
      this.trackRemoteProcessing(processing);
      const remote = await processing;
      if (!remote) return { status: "invalid" };
      if (
        isNewerCommunityThemeCoordinate(remote, {
          createdAt: this.lastRemoteCreatedAt,
          eventId: this.lastRemoteEventId,
        })
      ) {
        this.lastRemoteCreatedAt = remote.createdAt;
        this.lastRemoteEventId = remote.eventId;
        this.latestRemote = remote;
      }
      return { status: "valid", remote: this.latestRemote ?? remote };
    } catch {
      return { status: "unavailable" };
    }
  }

  publish(preference: CommunityThemePreference): void {
    if (this.destroyed) return;
    this.pending = preference;
    this.publishRetryAttempt = 0;
    this.schedulePublish(DEBOUNCE_MS);
  }

  private schedulePublish(delayMs: number): void {
    if (this.destroyed) return;
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
    }
    this.debounceTimer = window.setTimeout(() => {
      this.debounceTimer = null;
      this.startPublish();
    }, delayMs);
  }

  private startPublish(): void {
    if (this.destroyed || this.publishInFlight || !this.pending) return;
    this.publishInFlight = true;
    const preference = this.pending;
    void this.doPublish(preference).finally(() => {
      this.publishInFlight = false;
      if (
        !this.destroyed &&
        this.pending &&
        !sameCommunityThemePreference(this.pending, preference) &&
        this.debounceTimer === null
      ) {
        this.schedulePublish(0);
      }
    });
  }

  getPending(): CommunityThemePreference | null {
    return this.pending;
  }

  acceptRemote(remote: RemoteCommunityTheme): void {
    if (
      isNewerCommunityThemeCoordinate(remote, {
        createdAt: this.lastRemoteCreatedAt,
        eventId: this.lastRemoteEventId,
      })
    ) {
      this.lastRemoteCreatedAt = remote.createdAt;
      this.lastRemoteEventId = remote.eventId;
      this.latestRemote = remote;
    }
    const lastPublished = this.lastPublished;
    if (
      lastPublished &&
      (lastPublished.createdAt !== remote.createdAt ||
        lastPublished.eventId !== remote.eventId)
    ) {
      this.lastPublished = null;
    }
  }

  cancelPendingPublish(): void {
    if (this.debounceTimer !== null) {
      window.clearTimeout(this.debounceTimer);
      this.debounceTimer = null;
    }
    this.pending = null;
  }

  private async doPublish(preference: CommunityThemePreference): Promise<void> {
    try {
      const lastPublished = this.lastPublished;
      if (
        this.destroyed ||
        (lastPublished &&
          sameCommunityThemePreference(lastPublished.preference, preference))
      ) {
        if (
          this.pending &&
          sameCommunityThemePreference(this.pending, preference)
        ) {
          this.pending = null;
          if (lastPublished) this.onPublished(lastPublished);
        }
        return;
      }
      const ciphertext = await nip44EncryptToSelf(JSON.stringify(preference));
      if (this.destroyed) return;
      const event = await signRelayEvent({
        kind: KIND_COMMUNITY_THEME,
        content: ciphertext,
        createdAt: Math.max(
          Math.floor(Date.now() / 1_000),
          this.lastRemoteCreatedAt + 1,
        ),
        tags: [
          ["d", D_TAG],
          ["t", D_TAG],
        ],
      });
      if (this.destroyed) return;
      await relayClient.publishEvent(
        event,
        "Timed out publishing community theme.",
        "Failed to publish community theme.",
      );
      await this.waitForDeliveredRemotes();
      if (this.destroyed) return;
      const published = {
        preference,
        createdAt: event.created_at,
        eventId: event.id,
      };
      const eventLostToRemote = isNewerCommunityThemeCoordinate(
        {
          createdAt: this.lastRemoteCreatedAt,
          eventId: this.lastRemoteEventId,
        },
        published,
      );
      if (eventLostToRemote) {
        this.lastPublished = null;
        this.publishRetryAttempt = 0;
        if (
          this.pending &&
          sameCommunityThemePreference(this.pending, preference)
        ) {
          this.schedulePublish(0);
        }
        return;
      }
      this.lastRemoteCreatedAt = event.created_at;
      this.lastRemoteEventId = event.id;
      this.lastPublished = published;
      this.publishRetryAttempt = 0;
      if (
        this.pending &&
        sameCommunityThemePreference(this.pending, preference)
      ) {
        this.pending = null;
      }
      this.onPublished(published);
    } catch (error) {
      console.warn("[communityThemeSync] publish failed:", error);
      if (
        this.destroyed ||
        !this.pending ||
        !sameCommunityThemePreference(this.pending, preference)
      ) {
        return;
      }
      const delay = Math.min(
        PUBLISH_RETRY_BASE_MS * 2 ** this.publishRetryAttempt,
        PUBLISH_RETRY_MAX_MS,
      );
      this.publishRetryAttempt += 1;
      this.schedulePublish(delay);
    }
  }

  private trackRemoteProcessing(task: Promise<unknown>): void {
    this.remoteProcessing.add(task);
    void task.then(
      () => this.remoteProcessing.delete(task),
      () => this.remoteProcessing.delete(task),
    );
  }

  private async waitForDeliveredRemotes(): Promise<void> {
    await Promise.allSettled([...this.remoteProcessing]);
  }

  async subscribeAndFetch(
    onUpdate: (remote: RemoteCommunityTheme) => void,
  ): Promise<{
    result: CommunityThemeHydrationResult;
    unsubscribe: () => Promise<void>;
  }> {
    // Establish live delivery before querying history. A relay can deliver a
    // replacement while an empty history query is in flight; subscribing
    // second would leave a blind spot where the controller seeds defaults over
    // an existing preference.
    let unsubscribe: () => Promise<void> = async () => {};
    const readiness = {
      value: "failed" as "eose" | "closed" | "timeout" | "failed",
    };
    const setReadiness = (result: "eose" | "closed" | "timeout") => {
      readiness.value = result;
    };
    try {
      unsubscribe = await relayClient.subscribeLive(
        {
          kinds: [KIND_COMMUNITY_THEME],
          authors: [this.pubkey],
          "#d": [D_TAG],
          limit: 0,
        },
        (event: RelayEvent) => this.processLiveEvent(event, onUpdate),
        setReadiness,
      );
    } catch {
      // History still provides a safe snapshot when live setup is temporarily
      // unavailable; reconnect catch-up can still restore later relay state.
    }
    const result = await this.fetchRemote();
    await this.waitForDeliveredRemotes();
    let hydrationResult: CommunityThemeHydrationResult;
    if (this.latestRemote) {
      hydrationResult = { status: "valid", remote: this.latestRemote };
    } else if (result.status === "valid") {
      hydrationResult = result;
    } else if (result.status === "invalid") {
      hydrationResult = { status: "invalid" };
    } else if (result.status === "unavailable") {
      hydrationResult = { status: "unavailable" };
    } else if (this.observedInvalidLiveRemote) {
      hydrationResult = { status: "invalid" };
    } else if (readiness.value === "eose") {
      hydrationResult = { status: "confirmed-absent" };
    } else {
      hydrationResult = { status: "unavailable" };
    }
    return { result: hydrationResult, unsubscribe };
  }

  private processLiveEvent(
    event: RelayEvent,
    onUpdate: (remote: RemoteCommunityTheme) => void,
  ): void {
    if (event.pubkey !== this.pubkey || this.destroyed) return;
    const processing = decryptAndParse(event).then((remote) => {
      if (this.destroyed) return;
      if (!remote) {
        this.observedInvalidLiveRemote = true;
        return;
      }
      if (
        isNewerCommunityThemeCoordinate(remote, {
          createdAt: this.lastRemoteCreatedAt,
          eventId: this.lastRemoteEventId,
        })
      ) {
        this.lastRemoteCreatedAt = remote.createdAt;
        this.lastRemoteEventId = remote.eventId;
        this.latestRemote = remote;
      }
      onUpdate(remote);
    });
    this.trackRemoteProcessing(processing);
  }

  async subscribe(
    onUpdate: (remote: RemoteCommunityTheme) => void,
  ): Promise<() => Promise<void>> {
    return relayClient.subscribeLive(
      {
        kinds: [KIND_COMMUNITY_THEME],
        authors: [this.pubkey],
        "#d": [D_TAG],
        limit: 0,
      },
      (event: RelayEvent) => this.processLiveEvent(event, onUpdate),
    );
  }

  destroy(): void {
    this.destroyed = true;
    this.cancelPendingPublish();
  }
}
