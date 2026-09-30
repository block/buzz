import type { QueryClient } from "@tanstack/react-query";

import {
  getAvatarPresentation,
  subscribeAvatarPresentations,
  type AvatarPresentation,
} from "@/features/profile/avatarPresentationStore";
import { refreshProfileCaches } from "@/features/profile/profileCacheSync";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { updateProfileAtRelay } from "@/shared/api/tauriProfiles";
import type { Profile } from "@/shared/api/types";
import { isRelayUnreachableError } from "@/shared/lib/relayError";

const AVATAR_SAVE_RETRY_DELAYS_MS = [5_000, 30_000, 120_000] as const;

type PendingAvatarSave = {
  avatarUrl: string;
  relayUrl: string;
  expectedPubkey: string;
  expectedAvatarUrl: string | null;
};

type DeferredAvatarSave = Pick<PendingAvatarSave, "avatarUrl" | "relayUrl">;

type AvatarSaveRegistration = {
  cancel: () => void;
  release: (
    input: Pick<PendingAvatarSave, "expectedPubkey" | "expectedAvatarUrl">,
  ) => void;
};

type AvatarProfileSyncDependencies = {
  getPresentation: (avatarUrl: string) => AvatarPresentation | null;
  subscribe: (listener: () => void) => () => void;
  saveProfile: (input: PendingAvatarSave) => Promise<Profile>;
  getActivePubkey: () => Promise<string | null>;
  refreshCaches: (profile: Profile, input: PendingAvatarSave) => Promise<void>;
  scheduleRetry?: (callback: () => void, delayMs: number) => () => void;
};

function isRetryableAvatarSaveError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    isRelayUnreachableError(error) || message.startsWith("relay rate-limited:")
  );
}

export function createAvatarProfileSync(
  dependencies: AvatarProfileSyncDependencies,
) {
  const pendingSyncs = new Map<string, () => void>();
  let generation = 0;

  const reset = () => {
    generation += 1;
    for (const stop of pendingSyncs.values()) stop();
    pendingSyncs.clear();
  };

  const queueSave = (input: PendingAvatarSave, assumeReady = false): void => {
    const syncKey = `${input.relayUrl}:${input.expectedPubkey}:${input.avatarUrl}`;
    if (pendingSyncs.has(syncKey)) return;

    let isSaving = false;
    let isReady = assumeReady;
    let retryAttempt = 0;
    let cancelRetry: (() => void) | null = null;
    let unsubscribe = () => {};
    const queuedGeneration = generation;
    const stop = () => {
      cancelRetry?.();
      cancelRetry = null;
      unsubscribe();
      pendingSyncs.delete(syncKey);
    };
    const saveIfReady = () => {
      if (generation !== queuedGeneration || cancelRetry !== null) return;
      const presentation = dependencies.getPresentation(input.avatarUrl);
      if (presentation?.state === "ready") isReady = true;
      if (!presentation && !isReady) {
        stop();
        return;
      }
      if (!isReady || isSaving) return;

      isSaving = true;
      void dependencies
        .saveProfile(input)
        .then(async (profile) => {
          if (generation !== queuedGeneration) return;
          const activePubkey = await dependencies.getActivePubkey();
          if (
            generation !== queuedGeneration ||
            activePubkey?.toLowerCase() !== input.expectedPubkey.toLowerCase()
          ) {
            return;
          }
          await dependencies.refreshCaches(profile, input);
        })
        .then(stop)
        .catch((error: unknown) => {
          if (
            generation !== queuedGeneration ||
            !isRetryableAvatarSaveError(error)
          ) {
            stop();
            return;
          }
          const delayMs = AVATAR_SAVE_RETRY_DELAYS_MS[retryAttempt];
          if (delayMs === undefined) {
            stop();
            return;
          }
          retryAttempt += 1;
          isSaving = false;
          const scheduleRetry =
            dependencies.scheduleRetry ??
            ((callback, delay) => {
              const timeout = window.setTimeout(callback, delay);
              return () => window.clearTimeout(timeout);
            });
          cancelRetry = scheduleRetry(() => {
            cancelRetry = null;
            saveIfReady();
          }, delayMs);
        });
    };

    unsubscribe = dependencies.subscribe(saveIfReady);
    pendingSyncs.set(syncKey, stop);
    saveIfReady();
  };

  const registerWhenReady = (
    input: DeferredAvatarSave,
  ): AvatarSaveRegistration => {
    const registrationKey = `registration:${input.relayUrl}:${input.avatarUrl}`;
    if (pendingSyncs.has(registrationKey)) {
      return { cancel: () => {}, release: () => {} };
    }

    let observedReady = false;
    let active = true;
    const queuedGeneration = generation;
    const observe = () => {
      if (generation !== queuedGeneration) return;
      if (dependencies.getPresentation(input.avatarUrl)?.state === "ready") {
        observedReady = true;
      }
    };
    const unsubscribe = dependencies.subscribe(observe);
    const cancel = () => {
      if (!active) return;
      active = false;
      unsubscribe();
      pendingSyncs.delete(registrationKey);
    };
    pendingSyncs.set(registrationKey, cancel);
    observe();

    return {
      cancel,
      release: (completion) => {
        if (!active || generation !== queuedGeneration) return;
        cancel();
        queueSave({ ...input, ...completion }, observedReady);
      },
    };
  };

  return { registerWhenReady, reset, saveWhenReady: queueSave };
}

type ProfileCacheRefresh = {
  profile: Profile;
  input: PendingAvatarSave;
};

type ProfileCacheRefreshQueue = {
  enqueue: (refresh: ProfileCacheRefresh) => Promise<void>;
  reset: () => void;
  setClient: (client: QueryClient) => () => void;
};

export function createProfileCacheRefreshQueue(
  refresh: (
    client: QueryClient,
    profile: Profile,
    relayUrl: string,
  ) => Promise<void>,
): ProfileCacheRefreshQueue {
  let client: QueryClient | null = null;
  const pending = new Map<string, ProfileCacheRefresh>();
  const refreshKey = ({ profile, input }: ProfileCacheRefresh) =>
    `${input.relayUrl}:${profile.pubkey.toLowerCase()}`;

  const flush = (nextClient: QueryClient) => {
    const queued = [...pending.values()];
    pending.clear();
    for (const item of queued) {
      void refresh(nextClient, item.profile, item.input.relayUrl);
    }
  };

  return {
    enqueue: async (item) => {
      if (client) {
        await refresh(client, item.profile, item.input.relayUrl);
        return;
      }
      pending.set(refreshKey(item), item);
    },
    reset: () => pending.clear(),
    setClient: (nextClient) => {
      client = nextClient;
      flush(nextClient);
      return () => {
        if (client === nextClient) client = null;
      };
    },
  };
}

const profileCacheRefreshQueue =
  createProfileCacheRefreshQueue(refreshProfileCaches);

const avatarProfileSync = createAvatarProfileSync({
  getPresentation: getAvatarPresentation,
  subscribe: subscribeAvatarPresentations,
  saveProfile: updateProfileAtRelay,
  getActivePubkey: async () => {
    try {
      return (await getIdentity()).pubkey;
    } catch {
      return null;
    }
  },
  refreshCaches: async (profile, input) => {
    await profileCacheRefreshQueue.enqueue({ profile, input });
  },
});

export function setAvatarProfileSyncQueryClient(
  client: QueryClient,
): () => void {
  return profileCacheRefreshQueue.setClient(client);
}

export function registerAvatarWhenReady(
  input: DeferredAvatarSave,
): AvatarSaveRegistration {
  return avatarProfileSync.registerWhenReady(input);
}

export function saveAvatarWhenReady(input: PendingAvatarSave): void {
  avatarProfileSync.saveWhenReady(input);
}

export function resetAvatarProfileSync(): void {
  profileCacheRefreshQueue.reset();
  avatarProfileSync.reset();
}
