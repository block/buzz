import * as React from "react";
import { useQueryClient, type QueryStatus } from "@tanstack/react-query";
import { toast } from "sonner";

import {
  managedAgentsQueryKey,
  relayAgentsQueryKey,
} from "@/features/agents/hooks";
import { channelsQueryKey } from "@/features/channels/hooks";
import {
  ensureWelcomeChannel,
  hasEnsuredWelcomeChannel,
  markWelcomeChannelEnsured,
  notifyWelcomeChannelReady,
  rememberPendingWelcomeChannel,
} from "@/features/onboarding/welcome";
import {
  ensureWelcomeGuideIntro,
  getWelcomeGuideAgentPubkeys,
} from "@/features/onboarding/welcomeGuide";
import { useProfileQuery } from "@/features/profile/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  createChannel,
  getChannelMembers,
  getChannels,
  joinChannel,
  updateChannel,
} from "@/shared/api/tauri";

const DEFAULT_AUTO_JOIN_CHANNEL_NAME = "general";

export type ChannelInitResult = { ok: true } | { ok: false; reason: string };

async function autoJoinDefaultChannel(
  queryClient: ReturnType<typeof useQueryClient>,
): Promise<ChannelInitResult> {
  try {
    const channels = await getChannels();
    const target = channels.find(
      (channel) =>
        channel.name === DEFAULT_AUTO_JOIN_CHANNEL_NAME && !channel.isMember,
    );
    if (!target) {
      return { ok: true };
    }
    await joinChannel(target.id);
    await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    return { ok: true };
  } catch (error) {
    return {
      ok: false,
      reason:
        error instanceof Error
          ? error.message
          : "Failed to join the general channel",
    };
  }
}

export async function initializeWelcomeChannel(
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
    const allowedMemberPubkeys = await getWelcomeGuideAgentPubkeys(
      communityScope,
    ).catch(() => []);
    const welcomeChannel = await ensureWelcomeChannel(
      {
        createChannel,
        getChannelMembers,
        getChannels,
        updateChannel,
      },
      {
        allowedMemberPubkeys,
      },
    );
    let didInitializeWelcomeGuide = false;
    try {
      await ensureWelcomeGuideIntro(welcomeChannel.id, communityScope);
      didInitializeWelcomeGuide = true;
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
      ]);
    } catch (error) {
      console.warn("Failed to initialize Welcome guide.", error);
    }
    if (didInitializeWelcomeGuide) {
      markWelcomeChannelEnsured(pubkey, communityScope);
    }
    if (focus) {
      rememberPendingWelcomeChannel(welcomeChannel.id);
    }
    await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    if (focus) {
      notifyWelcomeChannelReady(welcomeChannel.id);
    }
    return { ok: true };
  } catch (error) {
    console.warn("Failed to initialize Welcome channel.", error);
    return {
      ok: false,
      reason:
        error instanceof Error
          ? error.message
          : "Failed to create the Welcome channel",
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
      profileStatus === "success" && profileHasEvent === true;

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
  const welcomeChannelCommunityScope = activeCommunity?.relayUrl ?? null;
  const welcomeChannelInitPromisesRef = React.useRef(
    new Map<string, Promise<ChannelInitResult>>(),
  );
  const [isCompletingWelcomeSetup, setIsCompletingWelcomeSetup] =
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
  const welcomeChannelFocusIntentRef = React.useRef(new Map<string, boolean>());
  const requestWelcomeChannel = React.useCallback(
    (focus: boolean): Promise<ChannelInitResult> => {
      if (!currentPubkey || !welcomeChannelCommunityScope) {
        return Promise.resolve({ ok: true });
      }

      const welcomeChannelInitKey = `${welcomeChannelCommunityScope}:${currentPubkey}`;
      const currentPromise = welcomeChannelInitPromisesRef.current.get(
        welcomeChannelInitKey,
      );
      if (currentPromise) {
        // A focus=true request must not be swallowed behind an in-flight
        // focus=false promise. Upgrade the intent: when the background
        // promise resolves, chain a focus-only follow-up.
        if (
          focus &&
          !welcomeChannelFocusIntentRef.current.get(welcomeChannelInitKey)
        ) {
          welcomeChannelFocusIntentRef.current.set(welcomeChannelInitKey, true);
          return currentPromise.then((result) => {
            if (!result.ok) return result;
            return initializeWelcomeChannel(queryClient, {
              focus: true,
              pubkey: currentPubkey,
              communityScope: welcomeChannelCommunityScope,
            });
          });
        }
        return currentPromise;
      }

      if (focus) {
        welcomeChannelFocusIntentRef.current.set(welcomeChannelInitKey, true);
      }
      const promise = initializeWelcomeChannel(queryClient, {
        focus,
        pubkey: currentPubkey,
        communityScope: welcomeChannelCommunityScope,
      });
      welcomeChannelInitPromisesRef.current.set(welcomeChannelInitKey, promise);
      void promise.finally(() => {
        welcomeChannelInitPromisesRef.current.delete(welcomeChannelInitKey);
        welcomeChannelFocusIntentRef.current.delete(welcomeChannelInitKey);
      });
      return promise;
    },
    [currentPubkey, queryClient, welcomeChannelCommunityScope],
  );

  React.useEffect(() => {
    if (
      onboardingGate.stage !== "ready" ||
      !currentPubkey ||
      !welcomeChannelCommunityScope ||
      !readOnboardingCompletion(currentPubkey) ||
      hasEnsuredWelcomeChannel(currentPubkey, welcomeChannelCommunityScope)
    ) {
      return;
    }

    void requestWelcomeChannel(false);
  }, [
    currentPubkey,
    onboardingGate.stage,
    requestWelcomeChannel,
    welcomeChannelCommunityScope,
  ]);

  const showWelcomeRetryToast = React.useCallback(
    (reason: string) => {
      toast.error("Couldn't set up the Welcome channel", {
        action: {
          label: "Retry",
          onClick: () => {
            void requestWelcomeChannel(true).then((result) => {
              if (!result.ok) {
                showWelcomeRetryToast(result.reason);
              }
            });
          },
        },
        description: reason,
      });
    },
    [requestWelcomeChannel],
  );

  const showGeneralRetryToast = React.useCallback(
    (reason: string) => {
      toast.error("Couldn't join #general", {
        action: {
          label: "Retry",
          onClick: () => {
            void autoJoinDefaultChannel(queryClient).then((result) => {
              if (!result.ok) {
                showGeneralRetryToast(result.reason);
              }
            });
          },
        },
        description: reason,
      });
    },
    [queryClient],
  );

  const completeAndShowWelcome = React.useCallback(() => {
    setIsCompletingWelcomeSetup(true);
    gateComplete();
    void Promise.all([
      requestWelcomeChannel(true),
      autoJoinDefaultChannel(queryClient),
    ])
      .then(([welcomeResult, autoJoinResult]) => {
        if (!welcomeResult.ok) {
          showWelcomeRetryToast(welcomeResult.reason);
        }
        if (!autoJoinResult.ok) {
          showGeneralRetryToast(autoJoinResult.reason);
        }
        return refreshChannelsCache(queryClient);
      })
      .finally(() => {
        setIsCompletingWelcomeSetup(false);
      });
  }, [
    gateComplete,
    queryClient,
    requestWelcomeChannel,
    showGeneralRetryToast,
    showWelcomeRetryToast,
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
            : isCompletingWelcomeSetup
              ? ("blocking" as const)
              : onboardingGate.stage,
  };
}
