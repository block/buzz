import * as React from "react";
import { type QueryStatus, useQueryClient } from "@tanstack/react-query";

import { useIdentityQuery } from "@/shared/api/hooks";

const MACHINE_ONBOARDING_COMPLETION_STORAGE_KEY =
  "buzz-machine-onboarding-complete.v2";
const LEGACY_ONBOARDING_COMPLETION_STORAGE_KEY = "buzz-onboarding-complete.v1";

type MachineOnboardingStage =
  | "blocking"
  | "keyring-locked"
  | "onboarding"
  | "ready"
  | "relaunch-required"
  | "reset-failed";

function completionKey(prefix: string, pubkey: string) {
  return `${prefix}:${pubkey}`;
}

export function readMachineOnboardingCompletion(pubkey: string | null) {
  if (typeof window === "undefined" || !pubkey) return false;
  return (
    window.localStorage.getItem(
      completionKey(MACHINE_ONBOARDING_COMPLETION_STORAGE_KEY, pubkey),
    ) === "true"
  );
}

function clearMachineOnboardingCompletion(pubkey: string | null) {
  if (typeof window === "undefined" || !pubkey) return;
  window.localStorage.removeItem(
    completionKey(MACHINE_ONBOARDING_COMPLETION_STORAGE_KEY, pubkey),
  );
}

function forceMachineOnboarding() {
  if (!import.meta.env.DEV || typeof window === "undefined") return false;
  return (
    new URL(window.location.href).searchParams.get("machineOnboarding") === "1"
  );
}

function migrateMachineOnboardingCompletion(
  pubkey: string,
  hasConfiguredCommunity: boolean,
  isSharedIdentity: boolean,
) {
  if (forceMachineOnboarding()) return false;
  if (readMachineOnboardingCompletion(pubkey)) return true;

  const completedLegacyOnboarding =
    window.localStorage.getItem(
      completionKey(LEGACY_ONBOARDING_COMPLETION_STORAGE_KEY, pubkey),
    ) === "true";
  if (
    !completedLegacyOnboarding &&
    !hasConfiguredCommunity &&
    !isSharedIdentity
  ) {
    return false;
  }

  window.localStorage.setItem(
    completionKey(MACHINE_ONBOARDING_COMPLETION_STORAGE_KEY, pubkey),
    "true",
  );
  return true;
}

function identitySettled(status: QueryStatus, isFetching: boolean) {
  return !isFetching && (status === "success" || status === "error");
}

export function useMachineOnboardingState({
  hasConfiguredCommunity,
  isSharedIdentity,
}: {
  hasConfiguredCommunity: boolean;
  isSharedIdentity: boolean;
}) {
  const queryClient = useQueryClient();
  const identityQuery = useIdentityQuery();
  const identity = identityQuery.data;
  const currentPubkey = identity?.pubkey ?? null;
  const identityLost = identity?.lost === true;
  const identityLocked = identity?.locked === true;
  const identityResetFailed = identity?.resetFailed === true;
  const [completedPubkey, setCompletedPubkey] = React.useState<string | null>(
    () =>
      currentPubkey &&
      !forceMachineOnboarding() &&
      readMachineOnboardingCompletion(currentPubkey)
        ? currentPubkey
        : null,
  );
  const [evaluatedPubkey, setEvaluatedPubkey] = React.useState<string | null>(
    null,
  );
  const continuingPubkeyRef = React.useRef<string | null>(null);
  const startupPubkeyRef = React.useRef<string | null>(null);
  const [bootedLost, setBootedLost] = React.useState(false);
  const [bootedLocked, setBootedLocked] = React.useState(false);

  React.useEffect(() => {
    if (
      identityQuery.status === "success" &&
      startupPubkeyRef.current === null
    ) {
      startupPubkeyRef.current = currentPubkey;
    }
  }, [currentPubkey, identityQuery.status]);
  React.useEffect(() => {
    if (identityLost) setBootedLost(true);
  }, [identityLost]);
  React.useEffect(() => {
    if (identityLocked) setBootedLocked(true);
  }, [identityLocked]);

  React.useEffect(() => {
    if (
      !currentPubkey ||
      currentPubkey !== startupPubkeyRef.current ||
      identityQuery.status !== "success" ||
      identityLost
    ) {
      return;
    }
    if (
      migrateMachineOnboardingCompletion(
        currentPubkey,
        hasConfiguredCommunity,
        isSharedIdentity,
      )
    ) {
      setCompletedPubkey(currentPubkey);
    }
    setEvaluatedPubkey(currentPubkey);
  }, [
    currentPubkey,
    hasConfiguredCommunity,
    identityLost,
    identityQuery.status,
    isSharedIdentity,
  ]);

  const complete = React.useCallback(
    (completedIdentityPubkey?: string) => {
      const pubkey = completedIdentityPubkey ?? currentPubkey;
      if (!pubkey) return;
      window.localStorage.setItem(
        completionKey(MACHINE_ONBOARDING_COMPLETION_STORAGE_KEY, pubkey),
        "true",
      );
      setCompletedPubkey(pubkey);
    },
    [currentPubkey],
  );

  const continueWithIdentity = React.useCallback((pubkey: string) => {
    continuingPubkeyRef.current = pubkey;
  }, []);

  const reopen = React.useCallback(() => {
    clearMachineOnboardingCompletion(currentPubkey);
    setCompletedPubkey((pubkey) => (pubkey === currentPubkey ? null : pubkey));
    setEvaluatedPubkey(currentPubkey);
  }, [currentPubkey]);

  const relaunchRequired =
    ((bootedLost && !identityLost) || (bootedLocked && !identityLocked)) &&
    identityQuery.status === "success";
  const hasCompletedCurrentPubkey =
    completedPubkey === currentPubkey ||
    (!forceMachineOnboarding() &&
      readMachineOnboardingCompletion(currentPubkey));

  let stage: MachineOnboardingStage;
  if (identityResetFailed && identityQuery.status === "success") {
    stage = "reset-failed";
  } else if (identityLocked && identityQuery.status === "success") {
    stage = "keyring-locked";
  } else if (relaunchRequired) {
    stage = "relaunch-required";
  } else if (identityLost && identityQuery.status === "success") {
    stage = "onboarding";
  } else if (identityQuery.status === "error") {
    stage = "ready";
  } else if (
    !identitySettled(
      identityQuery.status,
      identityQuery.fetchStatus === "fetching",
    ) ||
    !currentPubkey ||
    // Imported identities are published before the flow can advance to setup.
    // Keep that explicitly requested identity switch in onboarding; only the
    // startup identity needs the one-render evaluation gate above.
    (!hasCompletedCurrentPubkey &&
      evaluatedPubkey !== currentPubkey &&
      continuingPubkeyRef.current !== currentPubkey)
  ) {
    stage = "blocking";
  } else if (identityLost || !hasCompletedCurrentPubkey) {
    stage = "onboarding";
  } else {
    stage = "ready";
  }

  return {
    complete,
    continueWithIdentity,
    currentPubkey,
    identityLost,
    queryClient,
    reopen,
    stage,
  };
}
