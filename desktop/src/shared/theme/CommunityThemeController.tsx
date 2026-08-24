import { useCallback, useEffect, useLayoutEffect, useRef } from "react";
import { useCommunities } from "@/features/communities/useCommunities";
import { relayClient } from "@/shared/api/relayClient";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  cacheAndApplyCommunityTheme,
  captureCommunityThemeAppearanceSnapshot,
  clearCommunityThemeMigrationOutbox,
  clearCommunityThemeOutbox,
  communityThemeAppearanceFallback,
  communityThemeApplyExpectation,
  communityThemePersistenceAction,
  communityThemeScopeFallback,
  hasMigratedCommunityTheme,
  markCommunityThemeMigrated,
  readCommunityThemeCurrentAppearance,
  readCommunityThemeMigrationOutbox,
  readCommunityThemeOutbox,
  readCommunityThemePreference,
  refreshCommunityThemeCurrentAppearance,
  sameCommunityThemePreference,
  writeCommunityThemeMigrationOutbox,
  writeCommunityThemeOutbox,
  writeCommunityThemePreference,
  type CommunityThemeAppearance,
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
  const appearanceSnapshotRef = useRef<CommunityThemeAppearance | null>(null);
  const expectedAppliedRef = useRef<CommunityThemePreference | null>(null);
  const scopedPreferenceRef = useRef<CommunityThemePreference | null>(null);
  const lastRemoteRef = useRef({ createdAt: 0, eventId: "" });
  const initialPreferenceRef = useRef<CommunityThemePreference>({
    version: 1,
    theme: theme.selectedThemeName as CommunityThemePreference["theme"],
    accent: theme.accentColor,
    followSystem: theme.followSystem,
    glassBackground: theme.glassBackground,
    glassOpacity: theme.glassOpacity,
    prominentActiveTab: theme.prominentActiveTab,
  });

  const currentPreferenceRef = useRef<CommunityThemePreference>({
    version: 1,
    theme: theme.selectedThemeName as CommunityThemePreference["theme"],
    accent: theme.accentColor,
    followSystem: theme.followSystem,
    glassBackground: theme.glassBackground,
    glassOpacity: theme.glassOpacity,
    prominentActiveTab: theme.prominentActiveTab,
  });
  currentPreferenceRef.current = {
    version: 1,
    theme: theme.selectedThemeName as CommunityThemePreference["theme"],
    accent: theme.accentColor,
    followSystem: theme.followSystem,
    glassBackground: theme.glassBackground,
    glassOpacity: theme.glassOpacity,
    prominentActiveTab: theme.prominentActiveTab,
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
    // Capture the profile's pre-migration appearance once, before this mount
    // rewrites the global glass keys. Every community then inherits the same
    // former settings regardless of hydration order.
    const snapshot = captureCommunityThemeAppearanceSnapshot(
      pubkey,
      initialPreferenceRef.current,
    );
    appearanceSnapshotRef.current = snapshot;
    const currentAppearance = readCommunityThemeCurrentAppearance(
      pubkey,
      snapshot,
    );
    const appearanceFallback = communityThemeAppearanceFallback(snapshot);
    const scopeFallback: CommunityThemePreference = {
      ...communityThemeScopeFallback(
        hasMigratedCommunityTheme(pubkey),
        initialPreferenceRef.current,
      ),
      ...currentAppearance,
    };
    const local = readCommunityThemePreference(
      pubkey,
      relayUrl,
      appearanceFallback,
    );
    const dirty = readCommunityThemeOutbox(
      pubkey,
      relayUrl,
      appearanceFallback,
    );
    // Scope migration decides whether an entirely empty community inherits the
    // outer appearance. Appearance migration separately fills fields missing
    // from an existing three-field record.
    const scopedPreference = dirty ?? local ?? scopeFallback;
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
    // Reuse the snapshot captured by the layout effect above. Re-reading from
    // storage here would collapse to defaults when the snapshot write was
    // rejected (a full store), even though the layout effect already resolved
    // the correct in-session value.
    const snapshot = appearanceSnapshotRef.current;
    const appearanceFallback = communityThemeAppearanceFallback(snapshot);
    const currentAppearance = readCommunityThemeCurrentAppearance(
      pubkey,
      snapshot ?? appearanceFallback,
    );
    const local = readCommunityThemePreference(
      pubkey,
      relayUrl,
      appearanceFallback,
    );
    const durablePending = readCommunityThemeOutbox(
      pubkey,
      relayUrl,
      appearanceFallback,
    );
    // A value already cached for this relay is the safest fallback for a later
    // incomplete event. Without one, use the immutable pre-migration snapshot,
    // never the mutable current-choice fallback used only for absent scopes.
    const legacyFallback = durablePending ?? local ?? appearanceFallback;
    const scopeFallback: CommunityThemePreference = {
      ...communityThemeScopeFallback(
        hasMigratedCommunityTheme(pubkey),
        initialPreferenceRef.current,
      ),
      ...currentAppearance,
    };
    scopeRef.current = scope;
    lastRemoteRef.current = { createdAt: 0, eventId: "" };
    const manager = new CommunityThemeSyncManager(
      pubkey,
      (published) => {
        const last = lastRemoteRef.current;
        if (isNewerCommunityThemeCoordinate(published, last)) {
          lastRemoteRef.current = {
            createdAt: published.createdAt,
            eventId: published.eventId,
          };
        }
        clearCommunityThemeOutbox(
          pubkey,
          relayUrl,
          published.preference,
          legacyFallback,
        );
        clearCommunityThemeMigrationOutbox(pubkey, relayUrl);
      },
      () =>
        scopedPreferenceRef.current ??
        readCommunityThemeOutbox(pubkey, relayUrl, legacyFallback) ??
        readCommunityThemePreference(pubkey, relayUrl, legacyFallback) ??
        legacyFallback,
    );
    managerRef.current = manager;
    if (durablePending) {
      manager.publish(durablePending);
    } else {
      const migrationPending = readCommunityThemeMigrationOutbox(
        pubkey,
        relayUrl,
        legacyFallback,
      );
      if (migrationPending) manager.publish(migrationPending);
    }

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
      const dirty = readCommunityThemeOutbox(pubkey, relayUrl, legacyFallback);
      if (dirty) {
        manager.publish(dirty);
        return;
      }
      // Migration-only upgrades never outrank relay state. A newer complete
      // coordinate cancels the pending upgrade rather than being overwritten by
      // a legacy-derived republish two seconds later.
      clearCommunityThemeMigrationOutbox(pubkey, relayUrl);
      const migrationWasSubmitted = manager.cancelPendingPublish(
        remote.preference,
        remote,
      );
      cacheAndApplyCommunityTheme(
        pubkey,
        relayUrl,
        remote.preference,
        applyPreference,
      );
      scopedPreferenceRef.current = remote.preference;
      if (migrationWasSubmitted) {
        // Submission cannot be withdrawn. Reassert the accepted remote after
        // the stale migration settles so it remains the winning coordinate.
        writeCommunityThemeOutbox(pubkey, relayUrl, remote.preference);
        return;
      }
      if (
        remote.needsUpgrade &&
        writeCommunityThemeMigrationOutbox(pubkey, relayUrl, remote.preference)
      ) {
        manager.publish(remote.preference);
      }
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
          const seed =
            readCommunityThemeOutbox(pubkey, relayUrl, legacyFallback) ??
            readCommunityThemePreference(pubkey, relayUrl, legacyFallback) ??
            scopedPreferenceRef.current ??
            scopeFallback;
          writeCommunityThemePreference(pubkey, relayUrl, seed);
          writeCommunityThemeOutbox(pubkey, relayUrl, seed);
          markCommunityThemeMigrated(pubkey);
          manager.publish(seed);
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
        const pending = readCommunityThemeOutbox(
          pubkey,
          relayUrl,
          legacyFallback,
        );
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
      glassBackground: theme.glassBackground,
      glassOpacity: theme.glassOpacity,
      prominentActiveTab: theme.prominentActiveTab,
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
    const stored = readCommunityThemePreference(
      pubkey,
      relayUrl,
      communityThemeAppearanceFallback(appearanceSnapshotRef.current),
    );
    if (stored && sameCommunityThemePreference(stored, preference)) return;
    // A genuine user edit that changes glass/opacity/prominent-tab updates the
    // profile-wide fallback used only for a genuinely absent community. Keep
    // that value separate from the immutable pre-migration snapshot used to
    // hydrate older three-field records, or one community's explicit choice
    // contaminates every still-legacy community.
    const priorScoped = scopedPreferenceRef.current;
    if (
      !priorScoped ||
      priorScoped.glassBackground !== preference.glassBackground ||
      priorScoped.glassOpacity !== preference.glassOpacity ||
      priorScoped.prominentActiveTab !== preference.prominentActiveTab
    ) {
      refreshCommunityThemeCurrentAppearance(pubkey, preference);
    }
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
    theme.glassBackground,
    theme.glassOpacity,
    theme.prominentActiveTab,
  ]);

  return null;
}
