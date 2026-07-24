import * as React from "react";
import { useQueryClient, type QueryStatus } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  managedAgentsQueryKey,
  relayAgentsQueryKey,
} from "@/features/agents/hooks";
import { channelsQueryKey } from "@/features/channels/hooks";
import {
  ensureStarterChannels,
  ensureWelcomeChannel,
  hasEnsuredWelcomeChannel,
  markWelcomeChannelEnsured,
  notifyWelcomeChannelReady,
  rememberPendingWelcomeChannel,
} from "@/features/onboarding/welcome";
import { forceFreshOnboarding } from "@/features/onboarding/devFreshOnboarding";
import { ensureWelcomeCanvas } from "@/features/onboarding/welcomeCanvas";
import { ensureWelcomeTeam } from "@/features/onboarding/welcomeGuide";
import { useProfileQuery } from "@/features/profile/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel } from "@/shared/api/types";
import {
  createChannel,
  deleteChannel,
  ensureStarterChannels as ensureStarterChannelsCommand,
  getChannelMembers,
  getChannels,
  updateChannel,
} from "@/shared/api/tauri";

const STARTER_CHANNEL_SETUP_TOAST_ID = "starter-channel-setup-error";

export type ChannelInitResult =
  | { ok: true; focusChannelId?: string }
  | { ok: false; reason: string; focusChannelId?: string };

const welcomeSeedPromises = new Map<string, Promise<void>>();

function seedWelcomeExperience(
  queryClient: ReturnType<typeof useQueryClient>,
  channelId: string,
  pubkey: string | null,
  communityScope: string | null,
) {
  const key = `${communityScope ?? ""}:${channelId}`;
  const current = welcomeSeedPromises.get(key);
  if (current) return current;

  const promise = (async () => {
    try {
      await ensureWelcomeTeam(channelId, communityScope);
      await ensureWelcomeCanvas(channelId);
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
      ]);
      markWelcomeChannelEnsured(pubkey, communityScope);
    } catch (error) {
      console.warn("Failed to seed the private Welcome experience.", error);
    }
  })().finally(() => welcomeSeedPromises.delete(key));
  welcomeSeedPromises.set(key, promise);
  return promise;
}

export async function initializeStarterChannels(
  queryClient: ReturnType<typeof useQueryClient>,
  {
    focus,
    pubkey,
    communityScope,
  }: {
    focus: boolean;
    pubkey: string | null;
    communityScope: string | null;
  },
): Promise<ChannelInitResult> {
  try {
    let starterChannels: Awaited<
      ReturnType<typeof ensureStarterChannels>
    > | null = null;
    let starterChannelsError: unknown = null;
    try {
      starterChannels = await ensureStarterChannels({
        ensureStarterChannels: ensureStarterChannelsCommand,
        getChannels,
      });
    } catch (error) {
      starterChannelsError = error;
      console.warn("Failed to initialize public starter channels.", error);
    }

    const welcomeChannel = await ensureWelcomeChannel(
      {
        createChannel,
        deleteChannel,
        getChannelMembers,
        getChannels,
        updateChannel,
      },
      {
        replaceExisting: forceFreshOnboarding,
      },
    );

    const starterChannelList = starterChannels?.channels ?? [];
    queryClient.setQueryData<Channel[]>(channelsQueryKey, (channels = []) => {
      const ensuredIds = new Set(
        starterChannelList.map((channel) => channel.id),
      );
      ensuredIds.add(welcomeChannel.id);
      return [
        ...starterChannelList,
        ...(starterChannelList.some(
          (channel) => channel.id === welcomeChannel.id,
        )
          ? []
          : [welcomeChannel]),
        ...channels.filter((channel) => !ensuredIds.has(channel.id)),
      ];
    });
    void seedWelcomeExperience(
      queryClient,
      welcomeChannel.id,
      pubkey,
      communityScope,
    );
    await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    if (focus) {
      // Refreshing can briefly replace the optimistic cache with an older relay
      // snapshot. Reinsert the just-ensured channels before announcing focus so
      // the route can consume the pending private Welcome channel immediately.
      queryClient.setQueryData<Channel[]>(channelsQueryKey, (channels = []) => {
        const byId = new Map(
          [...channels, ...starterChannelList, welcomeChannel].map(
            (channel) => [channel.id, channel],
          ),
        );
        return [...byId.values()];
      });
      rememberPendingWelcomeChannel(welcomeChannel.id);
      notifyWelcomeChannelReady(welcomeChannel.id);
    }
    const focusChannelId = focus ? welcomeChannel.id : undefined;
    if (starterChannelsError) {
      return {
        ok: false,
        focusChannelId,
        reason:
          starterChannelsError instanceof Error
            ? starterChannelsError.message
            : "Failed to set up starter channels",
      };
    }
    return { ok: true, focusChannelId };
  } catch (error) {
    console.warn("Failed to initialize starter channels.", error);
    return {
      ok: false,
      reason:
        error instanceof Error
          ? error.message
          : "Failed to set up starter channels",
    };
  }
}

async function refreshChannelsCache(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  try {
    queryClient.setQueryData(channelsQueryKey, await getChannels());
  } catch {
    // The next mounted channels query can still retry; this cache refresh is
    // only here to avoid a blank Home flash after first-run setup.
  }

  await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
}

const ONBOARDING_COMPLETION_STORAGE_KEY = "buzz-onboarding-complete.v1";
type OnboardingGateStage = "blocking" | "onboarding" | "ready";

type UseFirstRunOnboardingGateOptions = {
  currentPubkey: string | null;
  identityIsFetching: boolean;
  identityLost: boolean;
  identityStatus: QueryStatus;
  isSharedIdentity: boolean;
  profileHasEvent: boolean | undefined;
  profileIsFetching: boolean;
  profileStatus: QueryStatus;
};

type OnboardingGateState = {
  currentPubkey: string | null;
  hasCompletedCurrentPubkey: boolean;
  hasSettledCurrentPubkey: boolean;
  isOpen: boolean;
};

function onboardingCompletionStorageKey(pubkey: string) {
  return `${ONBOARDING_COMPLETION_STORAGE_KEY}:${pubkey}`;
}

function readOnboardingCompletion(pubkey: string | null) {
  if (forceFreshOnboarding) {
    return false;
  }
  if (typeof window === "undefined" || !pubkey) {
    return false;
  }

  return (
    window.localStorage.getItem(onboardingCompletionStorageKey(pubkey)) ===
    "true"
  );
}

function createOnboardingGateState(pubkey: string | null): OnboardingGateState {
  const hasCompletedCurrentPubkey = readOnboardingCompletion(pubkey);

  return {
    currentPubkey: pubkey,
    hasCompletedCurrentPubkey,
    hasSettledCurrentPubkey: hasCompletedCurrentPubkey,
    isOpen: false,
  };
}

function resolveActiveGateState(
  gateState: OnboardingGateState,
  currentPubkey: string | null,
) {
  return gateState.currentPubkey === currentPubkey
    ? gateState
    : createOnboardingGateState(currentPubkey);
}

function updateActiveGateState(
  gateState: OnboardingGateState,
  currentPubkey: string | null,
  update: (activeGateState: OnboardingGateState) => OnboardingGateState,
) {
  return update(resolveActiveGateState(gateState, currentPubkey));
}

function isSettledQueryStatus(status: QueryStatus) {
  return status === "success" || status === "error";
}

function resolveOnboardingGateStage({
  currentPubkey,
  gateState,
  identityIsFetching,
  identityStatus,
}: {
  currentPubkey: string | null;
  gateState: OnboardingGateState;
  identityIsFetching: boolean;
  identityStatus: QueryStatus;
}): OnboardingGateStage {
  const isBlockingCurrentPubkey =
    currentPubkey !== null &&
    !gateState.hasCompletedCurrentPubkey &&
    (gateState.isOpen || !gateState.hasSettledCurrentPubkey);

  if (gateState.isOpen) {
    return "onboarding";
  }

  if (
    identityIsFetching ||
    !isSettledQueryStatus(identityStatus) ||
    isBlockingCurrentPubkey
  ) {
    return "blocking";
  }

  return "ready";
}

export function useFirstRunOnboardingGate({
  currentPubkey,
  identityIsFetching,
  identityLost,
  identityStatus,
  isSharedIdentity,
  profileHasEvent,
  profileIsFetching,
  profileStatus,
}: UseFirstRunOnboardingGateOptions) {
  const [gateState, setGateState] = React.useState<OnboardingGateState>(() =>
    createOnboardingGateState(currentPubkey),
  );
  const activeGateState = resolveActiveGateState(gateState, currentPubkey);
  const { hasCompletedCurrentPubkey, hasSettledCurrentPubkey } =
    activeGateState;

  React.useEffect(() => {
    setGateState((current) =>
      current.currentPubkey === currentPubkey
        ? current
        : createOnboardingGateState(currentPubkey),
    );
  }, [currentPubkey]);

  // When the backend signals "identity lost" (keyring was cleared after a
  // successful migration), force onboarding open immediately so the user can
  // re-import their nsec. This runs once, after identity settles.
  React.useEffect(() => {
    if (!identityLost || !currentPubkey || identityStatus !== "success") {
      return;
    }
    setGateState((current) =>
      updateActiveGateState(current, currentPubkey, (activeGateState) => ({
        ...activeGateState,
        hasCompletedCurrentPubkey: false,
        hasSettledCurrentPubkey: true,
        isOpen: true,
      })),
    );
  }, [currentPubkey, identityLost, identityStatus]);

  React.useEffect(() => {
    // Fast-path: shared identity worktrees have already onboarded in the
    // main checkout. Skip unconditionally without waiting for the relay
    // profile query. Guarded by !hasCompletedCurrentPubkey so it fires once.
    if (
      !forceFreshOnboarding &&
      isSharedIdentity &&
      currentPubkey &&
      identityStatus === "success" &&
      !hasCompletedCurrentPubkey
    ) {
      if (typeof window !== "undefined") {
        window.localStorage.setItem(
          onboardingCompletionStorageKey(currentPubkey),
          "true",
        );
      }
      setGateState((current) =>
        updateActiveGateState(current, currentPubkey, (activeGateState) => ({
          ...activeGateState,
          hasCompletedCurrentPubkey: true,
          hasSettledCurrentPubkey: true,
          isOpen: false,
        })),
      );
      return;
    }

    // Original guard — restored to simple form.
    if (hasSettledCurrentPubkey || !currentPubkey) {
      return;
    }

    if (identityStatus === "error") {
      setGateState((current) =>
        updateActiveGateState(current, currentPubkey, (activeGateState) => ({
          ...activeGateState,
          hasSettledCurrentPubkey: true,
        })),
      );
      return;
    }

    if (identityStatus !== "success") {
      return;
    }

    if (!isSettledQueryStatus(profileStatus) || profileIsFetching) {
      return;
    }

    // If the relay has a real kind:0 metadata event for this pubkey, the user
    // has previously completed onboarding (possibly on another machine or app
    // data directory). Skip the onboarding flow and mark as complete so they
    // go straight to the app.
    //
    // We gate on `hasProfileEvent` — a flag set by the Tauri backend when a
    // real kind:0 event was found — rather than any field value. This correctly
    // handles the case where a returning user's display_name is empty: the event
    // still exists, so onboarding is skipped. A missing event (new user, or no
    // kind:0 on the relay) always shows onboarding regardless of display_name.
    const hasExistingProfile =
      !forceFreshOnboarding &&
      profileStatus === "success" &&
      profileHasEvent === true;

    setGateState((current) =>
      updateActiveGateState(current, currentPubkey, (activeGateState) => {
        // Re-read localStorage here to handle the webkit2gtk WAL race: the
        // synchronous useState initializer may have run before the WAL was
        // merged into the main SQLite file, returning null for a flag that is
        // actually present. By the time this effect fires (identity + profile
        // settled), the WAL has had time to merge and the read is reliable.
        const hasCompletedAfterRecheck =
          readOnboardingCompletion(currentPubkey);
        const alreadyOnboarded =
          activeGateState.hasCompletedCurrentPubkey ||
          hasCompletedAfterRecheck ||
          hasExistingProfile;
        if (alreadyOnboarded && typeof window !== "undefined") {
          window.localStorage.setItem(
            onboardingCompletionStorageKey(currentPubkey),
            "true",
          );
        }
        return {
          ...activeGateState,
          hasCompletedCurrentPubkey: alreadyOnboarded,
          hasSettledCurrentPubkey: true,
          isOpen: !alreadyOnboarded,
        };
      }),
    );
  }, [
    currentPubkey,
    hasCompletedCurrentPubkey,
    hasSettledCurrentPubkey,
    identityStatus,
    isSharedIdentity,
    profileHasEvent,
    profileIsFetching,
    profileStatus,
  ]);

  const skipForNow = React.useCallback(() => {
    setGateState((current) =>
      updateActiveGateState(current, currentPubkey, (activeGateState) => ({
        ...activeGateState,
        hasSettledCurrentPubkey: true,
        isOpen: false,
      })),
    );
  }, [currentPubkey]);

  const complete = React.useCallback(() => {
    if (typeof window !== "undefined" && currentPubkey) {
      window.localStorage.setItem(
        onboardingCompletionStorageKey(currentPubkey),
        "true",
      );
    }

    setGateState({
      currentPubkey,
      hasCompletedCurrentPubkey: true,
      hasSettledCurrentPubkey: true,
      isOpen: false,
    });
  }, [currentPubkey]);

  return {
    complete,
    skipForNow,
    stage: resolveOnboardingGateStage({
      currentPubkey,
      gateState: activeGateState,
      identityIsFetching,
      identityStatus,
    }),
  };
}

export function useAppOnboardingState(isSharedIdentity: boolean) {
  const queryClient = useQueryClient();
  const { activeCommunity } = useCommunities();
  const identityQuery = useIdentityQuery();
  const identity = identityQuery.data;
  const currentPubkey = identity?.pubkey ?? null;
  const starterChannelsCommunityScope = activeCommunity?.relayUrl ?? null;
  const starterChannelsInitPromisesRef = React.useRef(
    new Map<string, Promise<ChannelInitResult>>(),
  );
  const [isCompletingStarterSetup, setIsCompletingStarterSetup] =
    React.useState(false);
  const identityLost = identity?.lost === true;
  // Keyring unreachable at boot — the real key is still in the OS keyring but
  // the session cannot access it. No in-app recovery is possible; the user
  // must unlock the keyring externally and relaunch. Mutually exclusive with lost.
  const identityLocked = identity?.locked === true;
  // Boot-time Phase 2 reset failed — wipe was attempted but verification failed.
  // The sentinel is preserved so the next relaunch retries automatically.
  const identityResetFailed = identity?.resetFailed === true;

  // Sticky boot fact: once identity was lost at boot, this remains true for the
  // entire session. Per-component state in OnboardingFlow cannot carry this
  // because the flow remounts when pubkey changes after recovery.
  const [bootedLost, setBootedLost] = React.useState(false);
  React.useEffect(() => {
    if (identityLost) setBootedLost(true);
  }, [identityLost]);

  // Sticky boot fact: once identity was locked at boot, this remains true for
  // the entire session. After import_identity clears the locked flag, the
  // relaunchRequired derivation uses this to force the relaunch screen.
  const [bootedLocked, setBootedLocked] = React.useState(false);
  React.useEffect(() => {
    if (identityLocked) setBootedLocked(true);
  }, [identityLocked]);

  const profileQuery = useProfileQuery(
    !identityLost && !identityLocked && identityQuery.status === "success",
  );
  const onboardingGate = useFirstRunOnboardingGate({
    currentPubkey,
    identityIsFetching: identityQuery.fetchStatus === "fetching",
    identityLost,
    identityStatus: identityQuery.status,
    isSharedIdentity,
    profileHasEvent: profileQuery.data?.hasProfileEvent,
    profileIsFetching: profileQuery.fetchStatus === "fetching",
    profileStatus: profileQuery.status,
  });
  const gateComplete = onboardingGate.complete;
  const starterChannelsFocusIntentRef = React.useRef(
    new Map<string, boolean>(),
  );
  const requestStarterChannels = React.useCallback(
    (focus: boolean): Promise<ChannelInitResult> => {
      if (!currentPubkey || !starterChannelsCommunityScope) {
        return Promise.resolve({ ok: true });
      }

      const starterChannelsInitKey = `${starterChannelsCommunityScope}:${currentPubkey}`;
      const currentPromise = starterChannelsInitPromisesRef.current.get(
        starterChannelsInitKey,
      );
      if (currentPromise) {
        // A focus=true request must not be swallowed behind an in-flight
        // focus=false promise. Upgrade the intent: when the background
        // promise resolves, chain a focus-only follow-up.
        if (
          focus &&
          !starterChannelsFocusIntentRef.current.get(starterChannelsInitKey)
        ) {
          starterChannelsFocusIntentRef.current.set(
            starterChannelsInitKey,
            true,
          );
          return currentPromise.then((result) => {
            if (!result.ok) return result;
            return initializeStarterChannels(queryClient, {
              focus: true,
              pubkey: currentPubkey,
              communityScope: starterChannelsCommunityScope,
            });
          });
        }
        return currentPromise;
      }

      if (focus) {
        starterChannelsFocusIntentRef.current.set(starterChannelsInitKey, true);
      }
      const promise = initializeStarterChannels(queryClient, {
        focus,
        pubkey: currentPubkey,
        communityScope: starterChannelsCommunityScope,
      }).finally(() => {
        starterChannelsInitPromisesRef.current.delete(starterChannelsInitKey);
        starterChannelsFocusIntentRef.current.delete(starterChannelsInitKey);
      });
      starterChannelsInitPromisesRef.current.set(
        starterChannelsInitKey,
        promise,
      );
      return promise;
    },
    [currentPubkey, queryClient, starterChannelsCommunityScope],
  );

  React.useEffect(() => {
    if (
      onboardingGate.stage !== "ready" ||
      !currentPubkey ||
      !starterChannelsCommunityScope ||
      !readOnboardingCompletion(currentPubkey) ||
      hasEnsuredWelcomeChannel(currentPubkey, starterChannelsCommunityScope)
    ) {
      return;
    }

    void requestStarterChannels(false);
  }, [
    currentPubkey,
    onboardingGate.stage,
    requestStarterChannels,
    starterChannelsCommunityScope,
  ]);

  const showStarterRetryToast = React.useCallback(
    (reason: string) => {
      toast.error("Couldn't set up starter channels", {
        id: STARTER_CHANNEL_SETUP_TOAST_ID,
        action: {
          label: "Retry",
          onClick: (event) => {
            event.preventDefault();
            void requestStarterChannels(true).then((result) => {
              if (!result.ok) {
                window.setTimeout(
                  // Sonner dismisses an action toast as its click resolves, so
                  // recreate a failed retry after that dismissal completes.
                  () => showStarterRetryToast(result.reason),
                  0,
                );
                return;
              }
              toast.dismiss(STARTER_CHANNEL_SETUP_TOAST_ID);
            });
          },
        },
        description: reason,
      });
    },
    [requestStarterChannels],
  );

  const completeAndShowWelcome = React.useCallback(() => {
    setIsCompletingStarterSetup(true);
    void requestStarterChannels(true).then(async (starterResult) => {
      await refreshChannelsCache(queryClient);
      gateComplete();
      setIsCompletingStarterSetup(false);
      if (starterResult.focusChannelId) {
        window.location.hash = `/channels/${encodeURIComponent(
          starterResult.focusChannelId,
        )}`;
      }
      if (!starterResult.ok) {
        showStarterRetryToast(starterResult.reason);
      }
    });
  }, [
    gateComplete,
    queryClient,
    requestStarterChannels,
    showStarterRetryToast,
  ]);
  const flow = {
    actions: {
      complete: completeAndShowWelcome,
      skipForNow: onboardingGate.skipForNow,
    },
    initialProfile: {
      profile: profileQuery.data,
    },
  };

  // Recovery completed this boot: force a relaunch screen regardless of any
  // other gate state. Backend startup routines (event sync, agent restore,
  // pending-event flush) were skipped for the ephemeral key and cannot restart
  // in-process, so nothing else can proceed until the app restarts.
  const relaunchRequired =
    ((bootedLost && !identityLost) || (bootedLocked && !identityLocked)) &&
    identityQuery.status === "success";

  return {
    currentPubkey,
    flow,
    identityLost,
    // reset-failed is the highest-precedence stage: a failed boot-time reset
    // means identity resolution was skipped entirely. Nothing can proceed until
    // the user relaunches and the wipe retries.
    stage:
      identityResetFailed && identityQuery.status === "success"
        ? ("reset-failed" as const)
        : identityLocked && identityQuery.status === "success"
          ? ("keyring-locked" as const)
          : relaunchRequired
            ? ("relaunch-required" as const)
            : isCompletingStarterSetup
              ? ("blocking" as const)
              : onboardingGate.stage,
  };
}
