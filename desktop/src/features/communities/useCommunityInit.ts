import { useEffect, useRef, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { isMacPlatform } from "@/shared/lib/platform";

import { relayClient } from "@/shared/api/relayClient";
import { resetRateLimitGate } from "@/shared/api/relayRateLimitGate";
import {
  autoConnectDefaultRelayEnabled,
  getDefaultRelayUrl,
} from "@/shared/api/tauri";
import { applyCommunity } from "@/shared/api/tauriWorkspace";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { clearTrayAgentActivity } from "@/shared/api/trayMenu";
import { getOverrides } from "@/shared/features";
import { resetMediaCaches } from "@/shared/lib/mediaUrl";
import { resetLinkPreviewMetadataCache } from "@/shared/lib/useResolvedLinkPreviews";
import { clearSearchHitEventCache } from "@/app/navigation/searchHitEventCache";
import { resetNavigationDeepLinkDrain } from "@/shared/deep-link";
import {
  clearAllDrafts,
  initDraftStore,
} from "@/features/messages/lib/useDrafts";
import { resetRenderScopedReactionHydration } from "@/features/messages/lib/renderScopedReactions";
import { resetAudioMediaLoadScheduler } from "@/features/messages/lib/audioMediaLoadScheduler";
import { resetBackgroundMediaUploads } from "@/features/messages/lib/backgroundMediaUploadStore";
import { resetLinkPreviewPreparations } from "@/features/messages/lib/linkPreviewPreparationStore";
import { resetPersistentAgentAudienceStore } from "@/features/messages/lib/persistentAgentAudience";
import {
  resetDetachedToastScope,
  setDetachedToastScope,
} from "@/features/messages/lib/detachedToastScope";
import {
  resetActiveAgentTurnsStore,
  saveActiveAgentTurnsForCommunity,
  restoreActiveAgentTurnsForCommunity,
} from "@/features/agents/activeAgentTurnsStore";
import { resetAgentWorkingSignal } from "@/features/agents/agentWorkingSignal";
import { resetAgentObserverStore } from "@/features/agents/observerRelayStore";
import { resetAvatarPresentations } from "@/features/profile/avatarPresentationStore";
import { resetAvatarProfileSync } from "@/features/profile/avatarProfileSync";
import { resetSidebarRelayConnectionCardState } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { clearMarkdownNodeCache } from "@/shared/ui/markdown/nodeCache";
import { resetMessageLinkMetadataCache } from "@/shared/ui/markdown/useMessageLinkMetadata";
import { resetVideoPlayerState } from "@/shared/ui/videoPlayerState";

import {
  initFirstCommunity,
  shouldAutoConnectDefaultRelay,
} from "./communityStorage";
import type { Community } from "./types";

/**
 * Tear down all community-scoped module singletons so the new
 * community starts with a clean slate. Hook-managed singletons
 * (e.g. ChannelMuteSyncManager, ChannelSectionSyncManager) are
 * destroyed via effect cleanup and do not need entries here.
 * See AGENTS.md "Community Switching" for the full contract.
 */
async function resetCommunityState({
  resetAvatarState,
}: {
  resetAvatarState: boolean;
}): Promise<void> {
  relayClient.disconnect();
  await resetNavigationDeepLinkDrain();
  resetRateLimitGate();
  clearAllDrafts();
  resetAgentObserverStore();
  resetActiveAgentTurnsStore();
  resetAgentWorkingSignal();
  if (isTauri() && isMacPlatform()) {
    void clearTrayAgentActivity();
  }
  if (resetAvatarState) {
    resetAvatarProfileSync();
    resetAvatarPresentations();
  }
  resetSidebarRelayConnectionCardState();
  resetMediaCaches();
  resetLinkPreviewMetadataCache();
  resetVideoPlayerState();
  resetAudioMediaLoadScheduler();
  resetRenderScopedReactionHydration();
  resetBackgroundMediaUploads();
  resetLinkPreviewPreparations();
  resetPersistentAgentAudienceStore();
  // Intentionally NOT reset: the in-flight detached agent-start map
  // (`useDetachedAgentStart`). Its entries are keyed by the scope each start
  // asserts (relay URL + signer + agent pubkey), so they cannot leak into the
  // new community, and they self-clean when the start settles. Clearing them
  // here is what permitted the A→B→A duplicate provider deploy: the backend's
  // scope assertion is a current-state check, so a start held across a
  // round-trip is valid again once A is re-applied — the map entry is its only
  // duplicate guard.
  resetDetachedToastScope();
  clearSearchHitEventCache();
  clearMarkdownNodeCache();
  resetMessageLinkMetadataCache();
}

type CommunityInitResult =
  | {
      isReady: true;
      needsSetup: false;
      appliedKey: string;
      identityPubkey: string | null;
    }
  | {
      isReady: false;
      needsSetup: true;
      defaultRelayUrl: string;
    }
  | { isReady: false; needsSetup: false; appliedKey: string | null }
  | { isReady: false; needsSetup: false; appliedKey: null; error: string };

/**
 * Applies the active community config to the Tauri backend and resets
 * all community-scoped module singletons when the community changes.
 *
 * Returns a discriminated union — only render the app after the
 * community is applied. When `needsSetup` is true, the caller
 * should show a first-run welcome screen.
 */
export function useCommunityInit(
  activeCommunity: Community | null,
  communityKey: string,
  isSharedIdentity: boolean,
  suppressAutoConnect = false,
): CommunityInitResult {
  const [result, setResult] = useState<CommunityInitResult>({
    isReady: false,
    needsSetup: false,
    appliedKey: null,
  });

  // Track whether this is the initial mount or a community switch.
  // On the initial mount we skip resetting singletons (they're fresh).
  const hasInitializedRef = useRef(false);

  // Track the previously-applied community ID so we can save its turn state
  // before resetting when the user switches to a different community.
  const prevCommunityIdRef = useRef<string | null>(null);
  // Deferred avatar work owns the relay captured when it was queued. A
  // same-relay reconnect during onboarding must not cancel that work, while an
  // actual relay boundary must clear both the queue and its presentation probe.
  const appliedRelayUrlRef = useRef<string | null>(null);
  // Deferred avatar work is also signed by the identity that queued it: a
  // same-relay identity replacement (in-app key import) must clear it so
  // pending profile writes are never published as the new signer.
  const appliedPubkeyRef = useRef<string | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: we intentionally depend on specific properties (id/relayUrl/token/reposDir) — depending on the whole object would trigger resets on name-only changes
  useEffect(() => {
    let cancelled = false;

    async function init() {
      if (!activeCommunity) {
        if (hasInitializedRef.current) {
          if (prevCommunityIdRef.current) {
            saveActiveAgentTurnsForCommunity(prevCommunityIdRef.current);
            prevCommunityIdRef.current = null;
          }
          try {
            await resetCommunityState({ resetAvatarState: true });
          } catch (error) {
            console.error("Failed to reset community state:", error);
            if (!cancelled) {
              setResult({
                isReady: false,
                needsSetup: false,
                appliedKey: null,
                error:
                  error instanceof Error
                    ? `Could not safely leave community: ${error.message}`
                    : "Could not safely leave community",
              });
            }
            return;
          }
          appliedRelayUrlRef.current = null;
          hasInitializedRef.current = false;
        }
        try {
          const defaultRelayUrl = await getDefaultRelayUrl();
          const autoConnectDefaultRelay =
            await autoConnectDefaultRelayEnabled();

          // Internal builds explicitly opt into treating their reviewed default
          // relay as the first community. Public builds retain community
          // selection even when BUZZ_RELAY_URL is overridden at runtime.
          if (
            !suppressAutoConnect &&
            (isSharedIdentity ||
              (autoConnectDefaultRelay &&
                shouldAutoConnectDefaultRelay(defaultRelayUrl)))
          ) {
            const identity = await getIdentity();
            if (cancelled) return;
            const community = initFirstCommunity(
              defaultRelayUrl,
              identity.pubkey,
            );
            if (community && !cancelled) {
              window.location.reload();
              return;
            }
            if (!cancelled) {
              setResult({
                isReady: false,
                needsSetup: true,
                defaultRelayUrl,
              });
            }
            return;
          }

          if (!cancelled) {
            setResult({
              isReady: false,
              needsSetup: true,
              defaultRelayUrl,
            });
          }
        } catch {
          if (!cancelled) {
            setResult({
              isReady: false,
              needsSetup: true,
              defaultRelayUrl: "ws://localhost:3000",
            });
          }
        }
        return;
      }

      // Mark this community config as pending while it is applied to the
      // backend. App.tsx also checks appliedKey against the active communityKey,
      // which prevents rendering community-scoped UI for a new community until
      // that exact config has finished applying.
      setResult({
        isReady: false,
        needsSetup: false,
        appliedKey: communityKey,
      });

      // Resolve the active signer before resetting singletons so
      // identity-scoped state is discarded when the pubkey changed, not only
      // when the relay boundary moved. The same value seeds the draft store
      // below. A failed read degrades exactly like the previous behavior:
      // relay-only reset semantics and an uninitialized draft store.
      let identityPubkey: string | null = null;
      try {
        identityPubkey = (await getIdentity()).pubkey;
      } catch (err) {
        console.error(
          "[useCommunityInit] getIdentity failed, draft store uninitialized:",
          err,
        );
      }
      if (cancelled) return;

      // On community switch (not initial mount), reset module singletons
      // so the new tree starts with a clean slate.
      if (hasInitializedRef.current) {
        // Save the outgoing community's turn state before wiping the store so
        // timers survive a round-trip (A → B → A keeps A's elapsed time).
        if (prevCommunityIdRef.current) {
          saveActiveAgentTurnsForCommunity(prevCommunityIdRef.current);
          // Null out immediately so a rapid community switch (A→B→C before
          // B's applyCommunity resolves) doesn't re-save the now-empty
          // store under the outgoing community ID and delete its snapshot.
          prevCommunityIdRef.current = null;
        }
        try {
          await resetCommunityState({
            resetAvatarState:
              appliedRelayUrlRef.current !== activeCommunity.relayUrl ||
              (identityPubkey !== null &&
                appliedPubkeyRef.current !== null &&
                appliedPubkeyRef.current !== identityPubkey),
          });
        } catch (error) {
          console.error("Failed to reset community state:", error);
          if (!cancelled) {
            setResult({
              isReady: false,
              needsSetup: false,
              appliedKey: null,
              error:
                error instanceof Error
                  ? `Could not safely switch communities: ${error.message}`
                  : "Could not safely switch communities",
            });
          }
          return;
        }
      }
      hasInitializedRef.current = true;
      appliedRelayUrlRef.current = activeCommunity.relayUrl;
      appliedPubkeyRef.current = identityPubkey ?? appliedPubkeyRef.current;

      // Apply community config to the Tauri backend.
      //
      // Note: we deliberately do NOT pass an nsec here. The persisted
      // `identity.key` file (resolved at startup by `resolve_persisted_identity`,
      // and updated atomically by `import_identity`) is the single source of
      // truth for the active key. Older builds stored the nsec in localStorage
      // and re-applied it on every reload, which silently overwrote any
      // imported key. `loadCommunities()` strips lingering `nsec` fields from
      // legacy entries; this site refuses to apply one even if present.
      try {
        await applyCommunity(
          activeCommunity.relayUrl,
          undefined,
          activeCommunity.token,
          activeCommunity.reposDir,
          getOverrides().agentManagedProfiles === true,
          getOverrides().threadScopedAcpSessions === true,
        );
      } catch (error) {
        // A bad `repos_dir` no longer reaches here — `apply_workspace` treats
        // it as non-fatal (relay/keys apply, bad value not persisted, REPOS
        // falls back to a real dir, a `repos-dir-error` toast surfaces it) and
        // returns Ok, so the app boots into a working state where the user can
        // fix the value in community settings. This catch now only fires on a
        // genuine relay/key apply failure (e.g. an invalid nsec or a poisoned
        // lock). For those, marking the community ready would render
        // community-scoped UI against a backend that never applied — park on
        // the loading gate (isReady:false, no appliedKey) instead.
        console.error("Failed to apply community to backend:", error);
        if (!cancelled) {
          setResult({
            isReady: false,
            needsSetup: false,
            appliedKey: null,
            error:
              error instanceof Error
                ? error.message
                : "Failed to apply community configuration",
          });
        }
        return;
      }

      if (!cancelled) {
        // Refresh relay-derived media state only after the backend has installed
        // this community's relay override. On cold launch, mediaUrl.ts may have
        // eagerly cached the default relay origin before applyCommunity ran;
        // leaving that stale value makes authenticated relay media look external
        // and bypass the localhost proxy.
        resetMediaCaches();

        if (identityPubkey !== null) {
          initDraftStore(identityPubkey, activeCommunity.relayUrl);
        }
        // Restore any turn state saved for this community (a prior A→B round-
        // trip). This runs after applyCommunity succeeds and before the app
        // renders so components see the restored timers on first render.
        restoreActiveAgentTurnsForCommunity(activeCommunity.id);
        // From here this community's UI is what renders, so warnings from
        // detached agent wakes captured under this scope may deliver again.
        setDetachedToastScope({
          relayUrl: activeCommunity.relayUrl,
          signerPubkey: identityPubkey,
        });
        // Prime the ref so the NEXT switch saves this community's state.
        prevCommunityIdRef.current = activeCommunity.id;
        setResult({
          isReady: true,
          needsSetup: false,
          appliedKey: communityKey,
          identityPubkey,
        });
      }
    }

    void init();

    return () => {
      cancelled = true;
    };
  }, [
    activeCommunity?.id,
    activeCommunity?.relayUrl,
    activeCommunity?.token,
    activeCommunity?.reposDir,
    isSharedIdentity,
    suppressAutoConnect,
    communityKey,
  ]);

  return result;
}
