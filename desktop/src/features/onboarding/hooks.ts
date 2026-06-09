import * as React from "react";
import { useQueryClient, type QueryStatus } from "@tanstack/react-query";

import { channelsQueryKey } from "@/features/channels/hooks";
import { useProfileQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { getChannels, joinChannel } from "@/shared/api/tauri";

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
    // Silent — auto-join is best-effort; the user can still find and join
    // the channel manually from the channel browser.
  }
}

const ONBOARDING_COMPLETION_STORAGE_KEY = "sprout-onboarding-complete.v1";
type OnboardingGateStage = "blocking" | "onboarding" | "ready";

type UseFirstRunOnboardingGateOptions = {
  currentPubkey: string | null;
  identityIsFetching: boolean;
  identityStatus: QueryStatus;
  isSharedIdentity: boolean;
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

    setGateState((current) =>
      updateActiveGateState(current, currentPubkey, (activeGateState) => {
        const alreadyOnboarded = activeGateState.hasCompletedCurrentPubkey;
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
  const identityQuery = useIdentityQuery();
  const identity = identityQuery.data;
  const currentPubkey = identity?.pubkey ?? null;
  const profileQuery = useProfileQuery();
  const onboardingGate = useFirstRunOnboardingGate({
    currentPubkey,
    identityIsFetching: identityQuery.fetchStatus === "fetching",
    identityStatus: identityQuery.status,
    isSharedIdentity,
    profileIsFetching: profileQuery.fetchStatus === "fetching",
    profileStatus: profileQuery.status,
  });
  const gateComplete = onboardingGate.complete;
  const completeAndAutoJoin = React.useCallback(() => {
    gateComplete();
    void autoJoinDefaultChannel(queryClient);
  }, [gateComplete, queryClient]);
  const flow = {
    actions: {
      complete: completeAndAutoJoin,
      skipForNow: onboardingGate.skipForNow,
    },
    initialProfile: {
      profile: profileQuery.data,
    },
  };

  return {
    currentPubkey,
    flow,
    stage: onboardingGate.stage,
  };
}
