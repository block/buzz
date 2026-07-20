import { useEffect, useRef, useState } from "react";

import { relayClient } from "@/shared/api/relayClient";
import { applyCommunity, getDefaultRelayUrl } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { getOverrides } from "@/shared/features";
import { resetMediaCaches } from "@/shared/lib/mediaUrl";
import { clearSearchHitEventCache } from "@/app/navigation/searchHitEventCache";
import {
  clearAllDrafts,
  initDraftStore,
} from "@/features/messages/lib/useDrafts";
import { resetRenderScopedReactionHydration } from "@/features/messages/lib/renderScopedReactions";
import {
  resetActiveAgentTurnsStore,
  saveActiveAgentTurnsForCommunity,
  restoreActiveAgentTurnsForCommunity,
} from "@/features/agents/activeAgentTurnsStore";
import { resetAgentWorkingSignal } from "@/features/agents/agentWorkingSignal";
import { resetAgentObserverStore } from "@/features/agents/observerRelayStore";
import { resetSidebarRelayConnectionCardState } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { clearMarkdownNodeCache } from "@/shared/ui/markdown/nodeCache";
import { resetVideoPlayerState } from "@/shared/ui/videoPlayerState";

import { initFirstCommunity } from "./communityStorage";
import type { Community } from "./types";

/**
 * Tear down all community-scoped module singletons so the new
 * community starts with a clean slate. Hook-managed singletons
 * (e.g. ChannelMuteSyncManager, ChannelSectionSyncManager) are
 * destroyed via effect cleanup and do not need entries here.
 * See AGENTS.md "Community Switching" for the full contract.
 */
function resetCommunityState(): void {
  relayClient.disconnect();
  clearAllDrafts();
  resetAgentObserverStore();
  resetActiveAgentTurnsStore();
  resetAgentWorkingSignal();
  resetSidebarRelayConnectionCardState();
  resetMediaCaches();
  resetVideoPlayerState();
  resetRenderScopedReactionHydration();
  clearSearchHitEventCache();
  clearMarkdownNodeCache();
}

type CommunityInitResult =
  | { isReady: true; needsSetup: false; appliedKey: string }
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

  // biome-ignore lint/correctness/useExhaustiveDependencies: we intentionally depend on specific properties (id/relayUrl/token/reposDir) — depending on the whole object would trigger resets on name-only changes
  useEffect(() => {
    let cancelled = false;

    async function init() {
      if (!activeCommunity) {
        try {
          const defaultRelayUrl = await getDefaultRelayUrl();

          if (isSharedIdentity) {
            const identity = await getIdentity();
            if (cancelled) return;
            initFirstCommunity(defaultRelayUrl, identity.pubkey);
            if (!cancelled) {
              window.location.reload();
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
        resetCommunityState();
      }
      hasInitializedRef.current = true;

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

        try {
          const identity = await getIdentity();
          if (cancelled) return;
          initDraftStore(identity.pubkey, activeCommunity.relayUrl);
        } catch (err) {
          if (cancelled) return;
          console.error(
            "[useCommunityInit] getIdentity failed, draft store uninitialized:",
            err,
          );
        }
        // Restore any turn state saved for this community (a prior A→B round-
        // trip). This runs after applyCommunity succeeds and before the app
        // renders so components see the restored timers on first render.
        restoreActiveAgentTurnsForCommunity(activeCommunity.id);
        // Prime the ref so the NEXT switch saves this community's state.
        prevCommunityIdRef.current = activeCommunity.id;
        setResult({
          isReady: true,
          needsSetup: false,
          appliedKey: communityKey,
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
    communityKey,
  ]);

  return result;
}
