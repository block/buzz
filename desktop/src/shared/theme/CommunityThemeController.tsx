import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import { useCommunities } from "@/features/communities/useCommunities";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  DEFAULT_COMMUNITY_THEME,
  cacheAndApplyCommunityTheme,
  clearCommunityThemeOutbox,
  communityThemeApplyExpectation,
  communityThemePersistenceAction,
  communityThemeScopeFallback,
  hasMigratedCommunityTheme,
  markCommunityThemeMigrated,
  readCommunityThemeOutbox,
  readCommunityThemePreference,
  sameCommunityThemePreference,
  writeCommunityThemeOutbox,
  writeCommunityThemePreference,
  type CommunityThemePreference,
} from "./communityThemePreference";
import {
  CommunityThemeSyncManager,
  communityThemeHydrationRemote,
  isNewerCommunityThemeCoordinate,
  shouldSeedCommunityTheme,
  type RemoteCommunityTheme,
} from "./communityThemeSync";
import { useTheme } from "./ThemeProvider";

export function CommunityThemeController() {
  const { activeCommunity } = useCommunities();
  const identity = useIdentityQuery();
  const theme = useTheme();
  const pubkey = identity.data?.pubkey;
  const relayUrl = activeCommunity?.relayUrl;
  const managerRef = useRef<CommunityThemeSyncManager | null>(null);
  const scopeRef = useRef("");
  const expectedAppliedRef = useRef<CommunityThemePreference | null>(null);
  const scopedPreferenceRef = useRef<CommunityThemePreference | null>(null);
  const lastRemoteRef = useRef({ createdAt: 0, eventId: "" });
  const initialPreferenceRef = useRef<CommunityThemePreference>({
    version: 1,
    theme: theme.selectedThemeName as CommunityThemePreference["theme"],
    accent: theme.accentColor,
    followSystem: theme.followSystem,
  });

  const currentPreferenceRef = useRef<CommunityThemePreference>({
    version: 1,
    theme: theme.selectedThemeName as CommunityThemePreference["theme"],
    accent: theme.accentColor,
    followSystem: theme.followSystem,
  });
  currentPreferenceRef.current = {
    version: 1,
    theme: theme.selectedThemeName as CommunityThemePreference["theme"],
    accent: theme.accentColor,
    followSystem: theme.followSystem,
  };

  const applyPreference = useCallback(
    (preference: CommunityThemePreference) => {
      expectedAppliedRef.current = communityThemeApplyExpectation(
        preference,
        currentPreferenceRef.current,
      );
      theme.applyAppearance(preference);
    },
    [theme.applyAppearance],
  );

  useLayoutEffect(() => {
    if (!pubkey || !relayUrl) return;
    const local = readCommunityThemePreference(pubkey, relayUrl);
    const dirty = readCommunityThemeOutbox(pubkey, relayUrl);
    // Preserve the user's existing global appearance the first time this
    // feature sees their current community. Later missing/malformed target
    // records use the stable default so the previous community never leaks.
    const fallback = communityThemeScopeFallback(
      hasMigratedCommunityTheme(pubkey),
      initialPreferenceRef.current,
    );
    const scopedPreference = dirty ?? local ?? fallback;
    scopedPreferenceRef.current = scopedPreference;
    applyPreference(scopedPreference);
    // Initialization is programmatic even when the provider already exposes
    // this exact value. Mark it after applyPreference so its no-op optimization
    // cannot make the persistence effect mistake the fallback for a user edit.
    expectedAppliedRef.current = communityThemeApplyExpectation(
      scopedPreference,
      currentPreferenceRef.current,
      true,
    );
  }, [pubkey, relayUrl, applyPreference]);

  useEffect(() => {
    if (!pubkey || !relayUrl) return;
    const scope = `${pubkey}:${relayUrl}`;
    scopeRef.current = scope;
    lastRemoteRef.current = { createdAt: 0, eventId: "" };
    const manager = new CommunityThemeSyncManager(pubkey, (published) => {
      const last = lastRemoteRef.current;
      if (isNewerCommunityThemeCoordinate(published, last)) {
        lastRemoteRef.current = {
          createdAt: published.createdAt,
          eventId: published.eventId,
        };
      }
      clearCommunityThemeOutbox(pubkey, relayUrl, published.preference);
    });
    managerRef.current = manager;
    const durablePending = readCommunityThemeOutbox(pubkey, relayUrl);
    if (durablePending) manager.publish(durablePending);

    const applyRemote = (remote: RemoteCommunityTheme) => {
      if (scopeRef.current !== scope) return;
      const last = lastRemoteRef.current;
      if (!isNewerCommunityThemeCoordinate(remote, last)) {
        return;
      }
      lastRemoteRef.current = {
        createdAt: remote.createdAt,
        eventId: remote.eventId,
      };
      manager.acceptRemote(remote);
      const dirty = readCommunityThemeOutbox(pubkey, relayUrl);
      if (dirty) {
        manager.publish(dirty);
        return;
      }
      scopedPreferenceRef.current = remote.preference;
      manager.cancelPendingPublish();
      cacheAndApplyCommunityTheme(
        pubkey,
        relayUrl,
        remote.preference,
        applyPreference,
      );
    };

    let unsubscribe: (() => Promise<void>) | null = null;
    void manager
      .subscribeAndFetch(applyRemote)
      .then(({ result, unsubscribe: dispose }) => {
        if (scopeRef.current !== scope) {
          void dispose();
          return;
        }
        unsubscribe = dispose;
        const remote = communityThemeHydrationRemote(result);
        if (remote) {
          applyRemote(remote);
          markCommunityThemeMigrated(pubkey);
        } else if (shouldSeedCommunityTheme(result)) {
          const local =
            readCommunityThemeOutbox(pubkey, relayUrl) ??
            readCommunityThemePreference(pubkey, relayUrl) ??
            scopedPreferenceRef.current ??
            DEFAULT_COMMUNITY_THEME;
          writeCommunityThemePreference(pubkey, relayUrl, local);
          writeCommunityThemeOutbox(pubkey, relayUrl, local);
          markCommunityThemeMigrated(pubkey);
          manager.publish(local);
        }
        // Invalid or unavailable hydration keeps the already-applied fallback
        // without publishing over relay state we could not establish safely.
      });
    const unsubscribeReconnect = relayClient.subscribeToReconnects(() => {
      void manager.fetchRemote().then((result) => {
        if (result.status === "valid") {
          applyRemote(result.remote);
          return;
        }
        if (result.status !== "absent") return;
        const pending = readCommunityThemeOutbox(pubkey, relayUrl);
        if (pending) manager.publish(pending);
      });
    });

    return () => {
      if (scopeRef.current === scope) scopeRef.current = "";
      manager.destroy();
      if (managerRef.current === manager) managerRef.current = null;
      unsubscribeReconnect();
      if (unsubscribe) void unsubscribe();
    };
  }, [pubkey, relayUrl, applyPreference]);

  useEffect(() => {
    if (!pubkey || !relayUrl) return;
    const preference: CommunityThemePreference = {
      version: 1,
      theme: theme.selectedThemeName as CommunityThemePreference["theme"],
      accent: theme.accentColor,
      followSystem: theme.followSystem,
    };
    const persistenceAction = communityThemePersistenceAction(
      expectedAppliedRef.current,
      preference,
    );
    if (persistenceAction === "defer") return;
    if (persistenceAction === "acknowledge") {
      expectedAppliedRef.current = null;
      return;
    }
    const stored = readCommunityThemePreference(pubkey, relayUrl);
    if (stored && sameCommunityThemePreference(stored, preference)) return;
    scopedPreferenceRef.current = preference;
    if (!writeCommunityThemePreference(pubkey, relayUrl, preference)) return;
    if (!writeCommunityThemeOutbox(pubkey, relayUrl, preference)) return;
    managerRef.current?.publish(preference);
  }, [
    pubkey,
    relayUrl,
    theme.selectedThemeName,
    theme.accentColor,
    theme.followSystem,
  ]);

  return null;
}
