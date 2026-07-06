import * as React from "react";
import { useQueryClient, type QueryStatus } from "@tanstack/react-query";

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
import { useWorkspaces } from "@/features/workspaces/useWorkspaces";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  createChannel,
  getChannelMembers,
  getChannels,
  joinChannel,
  updateChannel,
} from "@/shared/api/tauri";

const DEFAULT_AUTO_JOIN_CHANNEL_NAME = "general";

async function autoJoinDefaultChannel(
  queryClient: ReturnType<typeof useQueryClient>,
) {
  try {
    const channels = await getChannels();
    const target = channels.find(
      (channel) =>
        channel.name === DEFAULT_AUTO_JOIN_CHANNEL_NAME && !channel.isMember,
    );
    if (!target) {
      return;
    }
    await joinChannel(target.id);
    await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
  } catch {
    // Silent: auto-join is best-effort. The Welcome channel is created
    // separately, and users can still join channels manually from the browser.
  }
}

async function initializeWelcomeChannel(
  queryClient: ReturnType<typeof useQueryClient>,
  {
    focus,
    pubkey,
    workspaceScope,
  }: {
    focus: boolean;
    pubkey: string | null;
    workspaceScope: string | null;
  },
) {
  try {
    const allowedMemberPubkeys = await getWelcomeGuideAgentPubkeys(
      workspaceScope,
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
      await ensureWelcomeGuideIntro(welcomeChannel.id, workspaceScope);
      didInitializeWelcomeGuide = true;
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: managedAgentsQueryKey }),
        queryClient.invalidateQueries({ queryKey: relayAgentsQueryKey }),
      ]);
    } catch (error) {
      console.warn("Failed to initialize Welcome guide.", error);
    }
    if (didInitializeWelcomeGuide) {
      markWelcomeChannelEnsured(pubkey, workspaceScope);
    }
    if (focus) {
      rememberPendingWelcomeChannel(welcomeChannel.id);
    }
    await queryClient.invalidateQueries({ queryKey: channelsQueryKey });
    if (focus) {
      notifyWelcomeChannelReady(welcomeChannel.id);
    }
  } catch (error) {
    console.warn("Failed to initialize Welcome channel.", error);
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
  const { activeWorkspace } = useWorkspaces();
  const identityQuery = useIdentityQuery();
  const identity = identityQuery.data;
  const currentPubkey = identity?.pubkey ?? null;
  const welcomeChannelWorkspaceScope = activeWorkspace?.relayUrl ?? null;
  const welcomeChannelInitPromisesRef = React.useRef(
    new Map<string, Promise<void>>(),
  );
  const [isCompletingWelcomeSetup, setIsCompletingWelcomeSetup] =
    React.useState(false);
  const profileQuery = useProfileQuery();
  const onboardingGate = useFirstRunOnboardingGate({
    currentPubkey,
    identityIsFetching: identityQuery.fetchStatus === "fetching",
    identityStatus: identityQuery.status,
    isSharedIdentity,
    profileHasEvent: profileQuery.data?.hasProfileEvent,
    profileIsFetching: profileQuery.fetchStatus === "fetching",
    profileStatus: profileQuery.status,
  });
  const gateComplete = onboardingGate.complete;
  const requestWelcomeChannel = React.useCallback(
    (focus: boolean) => {
      if (!currentPubkey || !welcomeChannelWorkspaceScope) {
        return Promise.resolve();
      }

      const welcomeChannelInitKey = `${welcomeChannelWorkspaceScope}:${currentPubkey}`;
      const currentPromise = welcomeChannelInitPromisesRef.current.get(
        welcomeChannelInitKey,
      );
      if (currentPromise) {
        return currentPromise;
      }

      const promise = initializeWelcomeChannel(queryClient, {
        focus,
        pubkey: currentPubkey,
        workspaceScope: welcomeChannelWorkspaceScope,
      });
      welcomeChannelInitPromisesRef.current.set(welcomeChannelInitKey, promise);
      void promise.finally(() => {
        welcomeChannelInitPromisesRef.current.delete(welcomeChannelInitKey);
      });
      return promise;
    },
    [currentPubkey, queryClient, welcomeChannelWorkspaceScope],
  );

  React.useEffect(() => {
    if (
      onboardingGate.stage !== "ready" ||
      !currentPubkey ||
      !welcomeChannelWorkspaceScope ||
      !readOnboardingCompletion(currentPubkey) ||
      hasEnsuredWelcomeChannel(currentPubkey, welcomeChannelWorkspaceScope)
    ) {
      return;
    }

    void requestWelcomeChannel(false);
  }, [
    currentPubkey,
    onboardingGate.stage,
    requestWelcomeChannel,
    welcomeChannelWorkspaceScope,
  ]);

  const completeAndShowWelcome = React.useCallback(() => {
    setIsCompletingWelcomeSetup(true);
    gateComplete();
    void Promise.all([
      requestWelcomeChannel(true),
      autoJoinDefaultChannel(queryClient),
    ])
      .then(() => refreshChannelsCache(queryClient))
      .finally(() => {
        setIsCompletingWelcomeSetup(false);
      });
  }, [gateComplete, queryClient, requestWelcomeChannel]);
  const flow = {
    actions: {
      complete: completeAndShowWelcome,
      skipForNow: onboardingGate.skipForNow,
    },
    initialProfile: {
      profile: profileQuery.data,
    },
  };

  return {
    currentPubkey,
    flow,
    stage: isCompletingWelcomeSetup ? "blocking" : onboardingGate.stage,
  };
}
