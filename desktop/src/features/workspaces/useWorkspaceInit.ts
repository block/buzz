import { useEffect, useRef, useState } from "react";

import { relayClient } from "@/shared/api/relayClient";
import { applyWorkspace, getDefaultRelayUrl } from "@/shared/api/tauri";
import { getIdentity } from "@/shared/api/tauriIdentity";
import { resetMediaCaches } from "@/shared/lib/mediaUrl";
import { clearSearchHitEventCache } from "@/app/navigation/searchHitEventCache";
import {
  clearAllDrafts,
  initDraftStore,
} from "@/features/messages/lib/useDrafts";
import { resetRenderScopedReactionHydration } from "@/features/messages/lib/renderScopedReactions";
import {
  resetActiveAgentTurnsStore,
  saveActiveAgentTurnsForWorkspace,
  restoreActiveAgentTurnsForWorkspace,
} from "@/features/agents/activeAgentTurnsStore";
import { resetAgentWorkingSignal } from "@/features/agents/agentWorkingSignal";
import { resetAgentObserverStore } from "@/features/agents/observerRelayStore";
import { resetSidebarRelayConnectionCardState } from "@/features/sidebar/ui/useSidebarRelayConnectionCard";
import { clearMarkdownNodeCache } from "@/shared/ui/markdown/nodeCache";
import { resetVideoPlayerState } from "@/shared/ui/videoPlayerState";

import { initFirstWorkspace } from "./workspaceStorage";
import type { Workspace } from "./types";

/**
 * Tear down all workspace-scoped module singletons so the new
 * workspace starts with a clean slate. Hook-managed singletons
 * (e.g. ChannelMuteSyncManager, ChannelSectionSyncManager) are
 * destroyed via effect cleanup and do not need entries here.
 * See AGENTS.md "Workspace Switching" for the full contract.
 */
function resetWorkspaceState(): void {
  relayClient.disconnect();
  resetAgentObserverStore();
  resetActiveAgentTurnsStore();
  resetAgentWorkingSignal();
  resetSidebarRelayConnectionCardState();
  resetMediaCaches();
  resetVideoPlayerState();
  resetRenderScopedReactionHydration();
  clearSearchHitEventCache();
  clearAllDrafts();
  clearMarkdownNodeCache();
}

type WorkspaceInitResult =
  | { isReady: true; needsSetup: false; appliedKey: string }
  | {
      isReady: false;
      needsSetup: true;
      defaultRelayUrl: string;
    }
  | { isReady: false; needsSetup: false; appliedKey: string | null };

/**
 * Applies the active workspace config to the Tauri backend and resets
 * all workspace-scoped module singletons when the workspace changes.
 *
 * Returns a discriminated union — only render the app after the
 * workspace is applied. When `needsSetup` is true, the caller
 * should show a first-run welcome screen.
 */
export function useWorkspaceInit(
  activeWorkspace: Workspace | null,
  workspaceKey: string,
  isSharedIdentity: boolean,
): WorkspaceInitResult {
  const [result, setResult] = useState<WorkspaceInitResult>({
    isReady: false,
    needsSetup: false,
    appliedKey: null,
  });

  // Track whether this is the initial mount or a workspace switch.
  // On the initial mount we skip resetting singletons (they're fresh).
  const hasInitializedRef = useRef(false);

  // Track the previously-applied workspace ID so we can save its turn state
  // before resetting when the user switches to a different workspace.
  const prevWorkspaceIdRef = useRef<string | null>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: we intentionally depend on specific properties (id/relayUrl/token/reposDir) — depending on the whole object would trigger resets on name-only changes
  useEffect(() => {
    let cancelled = false;

    async function init() {
      if (!activeWorkspace) {
        try {
          const defaultRelayUrl = await getDefaultRelayUrl();

          if (isSharedIdentity) {
            const identity = await getIdentity();
            if (cancelled) return;
            initFirstWorkspace(defaultRelayUrl, identity.pubkey);
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

      // Mark this workspace config as pending while it is applied to the
      // backend. App.tsx also checks appliedKey against the active workspaceKey,
      // which prevents rendering workspace-scoped UI for a new workspace until
      // that exact config has finished applying.
      setResult({
        isReady: false,
        needsSetup: false,
        appliedKey: workspaceKey,
      });

      // On workspace switch (not initial mount), reset module singletons
      // so the new tree starts with a clean slate.
      if (hasInitializedRef.current) {
        // Save the outgoing workspace's turn state before wiping the store so
        // timers survive a round-trip (A → B → A keeps A's elapsed time).
        if (prevWorkspaceIdRef.current) {
          saveActiveAgentTurnsForWorkspace(prevWorkspaceIdRef.current);
          // Null out immediately so a rapid workspace switch (A→B→C before
          // B's applyWorkspace resolves) doesn't re-save the now-empty
          // store under the outgoing workspace ID and delete its snapshot.
          prevWorkspaceIdRef.current = null;
        }
        resetWorkspaceState();
      }
      hasInitializedRef.current = true;

      // Apply workspace config to the Tauri backend.
      //
      // Note: we deliberately do NOT pass an nsec here. The persisted
      // `identity.key` file (resolved at startup by `resolve_persisted_identity`,
      // and updated atomically by `import_identity`) is the single source of
      // truth for the active key. Older builds stored the nsec in localStorage
      // and re-applied it on every reload, which silently overwrote any
      // imported key. `loadWorkspaces()` strips lingering `nsec` fields from
      // legacy entries; this site refuses to apply one even if present.
      try {
        await applyWorkspace(
          activeWorkspace.relayUrl,
          undefined,
          activeWorkspace.token,
          activeWorkspace.reposDir,
        );
      } catch (error) {
        // A bad `repos_dir` no longer reaches here — `apply_workspace` treats
        // it as non-fatal (relay/keys apply, bad value not persisted, REPOS
        // falls back to a real dir, a `repos-dir-error` toast surfaces it) and
        // returns Ok, so the app boots into a working state where the user can
        // fix the value in workspace settings. This catch now only fires on a
        // genuine relay/key apply failure (e.g. an invalid nsec or a poisoned
        // lock). For those, marking the workspace ready would render
        // workspace-scoped UI against a backend that never applied — park on
        // the loading gate (isReady:false, no appliedKey) instead.
        console.error("Failed to apply workspace to backend:", error);
        if (!cancelled) {
          setResult({ isReady: false, needsSetup: false, appliedKey: null });
        }
        return;
      }

      if (!cancelled) {
        // Initialise the draft store for this identity so localStorage drafts
        // are scoped to the correct pubkey before the app renders.
        if (activeWorkspace.pubkey) {
          initDraftStore(activeWorkspace.pubkey);
        }
        // Restore any turn state saved for this workspace (a prior A→B round-
        // trip). This runs after applyWorkspace succeeds and before the app
        // renders so components see the restored timers on first render.
        restoreActiveAgentTurnsForWorkspace(activeWorkspace.id);
        // Prime the ref so the NEXT switch saves this workspace's state.
        prevWorkspaceIdRef.current = activeWorkspace.id;
        setResult({
          isReady: true,
          needsSetup: false,
          appliedKey: workspaceKey,
        });
      }
    }

    void init();

    return () => {
      cancelled = true;
    };
  }, [
    activeWorkspace?.id,
    activeWorkspace?.relayUrl,
    activeWorkspace?.token,
    activeWorkspace?.reposDir,
    isSharedIdentity,
    workspaceKey,
  ]);

  return result;
}
