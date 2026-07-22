import {
  getAvatarPresentation,
  subscribeAvatarPresentations,
  type AvatarPresentation,
} from "@/features/profile/avatarPresentationStore";
import { getProfile, updateProfile } from "@/shared/api/tauriProfiles";
import type { Profile } from "@/shared/api/types";

type AvatarProfileSyncDependencies = {
  getPresentation: (avatarUrl: string) => AvatarPresentation | null;
  getProfile: () => Promise<Profile>;
  subscribe: (listener: () => void) => () => void;
  updateProfile: (input: { avatarUrl: string }) => Promise<unknown>;
};

function normalizedAvatarUrl(avatarUrl: string | null | undefined) {
  return avatarUrl?.trim() || null;
}

export function createAvatarProfileSync(
  dependencies: AvatarProfileSyncDependencies,
) {
  const pendingSyncs = new Map<string, () => void>();
  let generation = 0;

  const reset = () => {
    generation += 1;
    for (const unsubscribe of pendingSyncs.values()) unsubscribe();
    pendingSyncs.clear();
  };

  const saveWhenReady = (
    avatarUrl: string,
    expectedPubkey: string,
    expectedAvatarUrl: string | null,
  ): void => {
    const syncKey = `${expectedPubkey}:${avatarUrl}`;
    if (pendingSyncs.has(syncKey)) return;

    const queuedGeneration = generation;
    let isSaving = false;
    const stop = () => {
      pendingSyncs.get(syncKey)?.();
      pendingSyncs.delete(syncKey);
    };
    const saveIfReady = () => {
      const presentation = dependencies.getPresentation(avatarUrl);
      if (!presentation) {
        stop();
        return;
      }
      if (presentation.state !== "ready" || isSaving) return;

      isSaving = true;
      void dependencies
        .getProfile()
        .then((profile) => {
          if (
            generation !== queuedGeneration ||
            profile.pubkey !== expectedPubkey ||
            normalizedAvatarUrl(profile.avatarUrl) !==
              normalizedAvatarUrl(expectedAvatarUrl)
          ) {
            return;
          }
          return dependencies.updateProfile({ avatarUrl });
        })
        .catch(() => undefined)
        .finally(stop);
    };

    pendingSyncs.set(syncKey, dependencies.subscribe(saveIfReady));
    saveIfReady();
  };

  return { reset, saveWhenReady };
}

const avatarProfileSync = createAvatarProfileSync({
  getPresentation: getAvatarPresentation,
  getProfile,
  subscribe: subscribeAvatarPresentations,
  updateProfile,
});

export function saveAvatarWhenReady(
  avatarUrl: string,
  expectedPubkey: string,
  expectedAvatarUrl: string | null,
): void {
  avatarProfileSync.saveWhenReady(avatarUrl, expectedPubkey, expectedAvatarUrl);
}

export function resetAvatarProfileSync(): void {
  avatarProfileSync.reset();
}
