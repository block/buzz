import * as React from "react";
import { flushSync } from "react-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  profileQueryKey,
  useUpdateProfileMutation,
} from "@/features/profile/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { getMyRelayMembershipLookup } from "@/shared/api/relayMembers";
import { savePendingProfile } from "@/features/profile/pendingProfileSave";
import {
  isRelayMembershipDeniedError,
  isRelayUnreachableError,
} from "@/shared/lib/relayError";
import {
  getIdentity,
  importIdentity,
  persistCurrentIdentity,
} from "@/shared/api/tauriIdentity";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { AvatarStep } from "./AvatarStep";
import { OnboardingChrome } from "./OnboardingChrome";
import { OnboardingFooterProvider } from "./OnboardingFooter";
import { MembershipDenied } from "./MembershipDenied";
import {
  NostrKeyImportForm,
  type NostrKeyImportStage,
} from "./NostrKeyImportForm";
import { useCommunities } from "@/features/communities/useCommunities";
import { CommunityChangeOverlay } from "@/features/communities/ui/CommunityChangeOverlay";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { ProfileStep } from "./ProfileStep";
import type {
  OnboardingActions,
  OnboardingPage,
  OnboardingProfileSeed,
  OnboardingProfileValues,
  ProfileStepState,
} from "./types";

type MembershipCheckResult = "denied" | "ok" | "unreachable" | "error";

async function checkMembershipStatus(): Promise<MembershipCheckResult> {
  try {
    const { membership, snapshotFound } = await getMyRelayMembershipLookup();
    if (snapshotFound && membership === null) return "denied";
    return "ok";
  } catch (error) {
    if (isRelayMembershipDeniedError(error)) return "denied";
    // Native Tauri commands report connectivity failures with the stable
    // "relay unreachable:" prefix (see desktop/src-tauri/src/relay.rs), which
    // the legacy browser-fetch substrings below do not match.
    if (isRelayUnreachableError(error)) return "unreachable";
    if (error instanceof Error) {
      const msg = error.message.toLowerCase();
      if (
        msg.includes("failed to fetch") ||
        msg.includes("networkerror") ||
        msg.includes("timeout") ||
        msg.includes("econnrefused") ||
        msg.includes("enotfound") ||
        msg.includes("connection") ||
        msg.includes("aborted")
      ) {
        return "unreachable";
      }
    }
    return "error";
  }
}

type OnboardingFlowProps = {
  actions: OnboardingActions;
  identityLost?: boolean;
  initialProfile: OnboardingProfileSeed;
};

function isFallbackDisplayName(value?: string | null) {
  const normalizedValue = value?.trim().toLowerCase() ?? "";
  return (
    normalizedValue.startsWith("npub1") ||
    normalizedValue.startsWith("nostr:npub1")
  );
}

function sanitizeDisplayName(value?: string | null) {
  const trimmedValue = value?.trim() ?? "";
  return isFallbackDisplayName(trimmedValue) ? "" : trimmedValue;
}

function resolveSavedProfile({
  profile,
}: OnboardingProfileSeed): OnboardingProfileValues {
  return {
    avatarUrl: profile?.avatarUrl ?? "",
    displayName: sanitizeDisplayName(profile?.displayName),
  };
}

function createProfileUpdatePayload({
  draftProfile,
  savedProfile,
}: {
  draftProfile: OnboardingProfileValues;
  savedProfile: OnboardingProfileValues;
}) {
  const nextDisplayName = draftProfile.displayName.trim();
  const nextAvatarUrl = draftProfile.avatarUrl.trim();
  const updatePayload: {
    avatarUrl?: string;
    displayName?: string;
  } = {};

  if (
    nextDisplayName.length > 0 &&
    nextDisplayName !== savedProfile.displayName
  ) {
    updatePayload.displayName = nextDisplayName;
  }

  if (nextAvatarUrl.length > 0 && nextAvatarUrl !== savedProfile.avatarUrl) {
    updatePayload.avatarUrl = nextAvatarUrl;
  }

  return updatePayload;
}

function resolveProfileSaveRecovery(
  errorMessage: string | null,
  savedDisplayName: string,
): ProfileStepState["saveRecovery"] {
  return {
    canAdvanceWithoutSaving:
      errorMessage !== null && savedDisplayName.length > 0,
    canSkipForNow: errorMessage !== null && savedDisplayName.length === 0,
    errorMessage,
  };
}

export function OnboardingFlow({
  actions,
  identityLost = false,
  initialProfile,
}: OnboardingFlowProps) {
  const { complete, skipForNow } = actions;
  const { activeCommunity } = useCommunities();
  const queryClient = useQueryClient();
  const savedProfile = resolveSavedProfile(initialProfile);
  const profileUpdateMutation = useUpdateProfileMutation();
  const { error: profileSaveError, isPending: isSavingProfile } =
    profileUpdateMutation;
  // When identity was lost (keyring cleared after migration), land the user
  // directly on the import step with a recovery notice rather than profile setup.
  const [currentPage, setCurrentPage] = React.useState<OnboardingPage>(
    identityLost ? "key-import" : "profile",
  );
  const [keyImportStage, setKeyImportStage] =
    React.useState<NostrKeyImportStage>("key-entry");
  const [isKeyImporting, setIsKeyImporting] = React.useState(false);
  const [keyImportFormKey, setKeyImportFormKey] = React.useState(0);
  const [profileDraft, setProfileDraft] =
    React.useState<OnboardingProfileValues>(savedProfile);
  const [deniedPubkey, setDeniedPubkey] = React.useState<string>("");
  const [persistError, setPersistError] = React.useState<string | null>(null);
  const [isUploadingAvatar, setIsUploadingAvatar] = React.useState(false);
  const [isProfileAdvancePending, setIsProfileAdvancePending] =
    React.useState(false);
  const [membershipRetryPage, setMembershipRetryPage] = React.useState<
    OnboardingPage | "complete"
  >("avatar");
  const [deniedFromPage, setDeniedFromPage] =
    React.useState<OnboardingPage>("profile");
  const [isCommunityChangeOpen, setIsCommunityChangeOpen] =
    React.useState(false);
  const [membershipError, setMembershipError] = React.useState<{
    kind: "unreachable" | "error";
    message?: string;
  } | null>(null);
  const [transitionDirection, setTransitionDirection] =
    React.useState<OnboardingTransitionDirection>("forward");
  const systemColorScheme = useSystemColorScheme();

  const resetProfileSaveError = React.useCallback(() => {
    profileUpdateMutation.reset();
  }, [profileUpdateMutation]);

  const updateProfileDraft = React.useCallback(
    (patch: Partial<OnboardingProfileValues>) => {
      resetProfileSaveError();
      setProfileDraft((current) => ({
        ...current,
        ...patch,
      }));
    },
    [resetProfileSaveError],
  );

  const showAvatarPage = React.useCallback(
    (direction: OnboardingTransitionDirection = "forward") => {
      setTransitionDirection(direction);
      setCurrentPage("avatar");
    },
    [],
  );

  const showProfilePage = React.useCallback(() => {
    setMembershipError(null);
    setTransitionDirection("backward");
    setCurrentPage("profile");
  }, []);

  const showKeyImportPage = React.useCallback(() => {
    setTransitionDirection("forward");
    setCurrentPage("key-import");
  }, []);

  /**
   * Route to the membership-denied screen, parking whatever the user typed so
   * it can be written once they belong to a relay.
   *
   * A kind:0 write requires membership, but first-run setup runs before the
   * user has joined anything — so this path is reachable with a perfectly
   * valid name and avatar in hand. Discarding them (and the old dead end) is
   * what made this a funnel-killer (block/buzz#3544).
   */
  const enterMembershipDenied = React.useCallback(
    async (nextPage: OnboardingPage | "complete") => {
      let pubkey = "";
      try {
        const identity = await getIdentity();
        pubkey = identity.pubkey;
      } catch {
        pubkey = "";
      }
      setDeniedPubkey(pubkey);
      if (pubkey) {
        savePendingProfile({
          pubkey,
          displayName: profileDraft.displayName.trim(),
          avatarUrl: profileDraft.avatarUrl.trim(),
        });
      }
      setDeniedFromPage((prev) =>
        currentPage === "membership-denied" ? prev : currentPage,
      );
      setMembershipRetryPage(nextPage);
      setCurrentPage("membership-denied");
    },
    [currentPage, profileDraft],
  );

  const saveProfileAndContinue = React.useCallback(
    async (nextPage: OnboardingPage | "complete") => {
      if (isProfileAdvancePending) {
        return;
      }
      if (profileDraft.displayName.trim().length === 0) {
        return;
      }

      flushSync(() => {
        setIsProfileAdvancePending(true);
      });

      try {
        // Check membership before attempting the profile save. On open relays
        // this passes instantly. On gated relays it prevents a 403 during save.
        const membershipStatus = await checkMembershipStatus();
        setMembershipError(null);

        if (membershipStatus === "denied") {
          await enterMembershipDenied(nextPage);
          return;
        }

        if (membershipStatus === "unreachable") {
          setMembershipError({ kind: "unreachable" });
          return;
        }

        if (membershipStatus === "error") {
          setMembershipError({
            kind: "error",
            message: "Server error — try again",
          });
          return;
        }

        const updatePayload = createProfileUpdatePayload({
          draftProfile: profileDraft,
          savedProfile,
        });

        if (Object.keys(updatePayload).length > 0) {
          try {
            await profileUpdateMutation.mutateAsync(updatePayload);
          } catch (error) {
            if (isRelayMembershipDeniedError(error)) {
              await enterMembershipDenied(nextPage);
              return;
            }

            // Error falls through to the error banner / recovery buttons.
            return;
          }
        }

        if (nextPage === "complete") {
          complete();
          return;
        }
        showAvatarPage();
      } finally {
        setIsProfileAdvancePending(false);
      }
    },
    [
      enterMembershipDenied,
      isProfileAdvancePending,
      profileDraft,
      profileUpdateMutation,
      savedProfile,
      complete,
      showAvatarPage,
    ],
  );

  const updateDisplayNameDraft = React.useCallback(
    (value: string) => {
      updateProfileDraft({ displayName: value });
    },
    [updateProfileDraft],
  );

  const updateAvatarUrlDraft = React.useCallback(
    (value: string) => {
      updateProfileDraft({ avatarUrl: value });
    },
    [updateProfileDraft],
  );

  const resetAvatarDraft = React.useCallback(() => {
    updateProfileDraft({ avatarUrl: savedProfile.avatarUrl });
  }, [savedProfile.avatarUrl, updateProfileDraft]);

  const advanceFromProfileWithoutSaving = React.useCallback(() => {
    profileUpdateMutation.reset();
    setProfileDraft((current) => ({
      ...current,
      displayName: savedProfile.displayName,
    }));
    showAvatarPage();
  }, [profileUpdateMutation, savedProfile.displayName, showAvatarPage]);

  const saveErrorMessage =
    profileSaveError instanceof Error ? profileSaveError.message : null;
  const profileStepState: ProfileStepState = {
    avatar: {
      draftUrl: profileDraft.avatarUrl,
      savedUrl: savedProfile.avatarUrl,
    },
    isUploadingAvatar,
    isSaving: isSavingProfile || isProfileAdvancePending,
    name: {
      draftValue: profileDraft.displayName,
      savedValue: savedProfile.displayName,
    },
    saveRecovery: resolveProfileSaveRecovery(
      saveErrorMessage,
      savedProfile.displayName,
    ),
  };
  const avatarStepState: ProfileStepState = {
    ...profileStepState,
    saveRecovery: saveErrorMessage
      ? {
          canAdvanceWithoutSaving: true,
          canSkipForNow: false,
          errorMessage: saveErrorMessage,
        }
      : profileStepState.saveRecovery,
  };
  // Machine-level identity, backup, and provider setup have already completed.
  // This relay-scoped flow now owns only the community profile.
  const activeSteps: OnboardingPage[] = ["profile", "avatar"];
  const STEP_OFFSET = 1;
  // key-import occupies the same position as profile.
  const normalizedPage: OnboardingPage =
    currentPage === "key-import" ? "profile" : currentPage;
  const pageIndex = activeSteps.indexOf(normalizedPage);
  const currentStep = pageIndex >= 0 ? pageIndex + STEP_OFFSET : STEP_OFFSET;
  const totalOnboardingSteps = activeSteps.length;

  // Swapping the identity changes the pubkey, which remounts this flow
  // (keyed on pubkey in App.tsx) and re-runs the onboarding gate: the new
  // key's relay profile reseeds the steps, and a key that already finished
  // onboarding on this machine skips straight into the app.
  const importExistingKey = React.useCallback(
    async (nsec: string, password?: string) => {
      const identity = await importIdentity(nsec, password);
      relayClient.disconnect();
      queryClient.setQueryData(["identity"], identity);
      queryClient.removeQueries({ queryKey: profileQueryKey });
      profileUpdateMutation.reset();
      setDeniedPubkey("");
      setTransitionDirection("backward");
      setCurrentPage("profile");
    },
    [profileUpdateMutation, queryClient],
  );

  // Lost-mode "start new identity": confirm first (irreversible), then persist
  // the ephemeral key so the new identity is durable, then let the stage
  // machinery (bootedLost + !identityLost) replace this flow with
  // RelaunchRequiredScreen. No navigation needed here.
  const handleLostModeBack = React.useCallback(async () => {
    const confirmed = window.confirm(
      "This will create a new identity and abandon your previous key. This cannot be undone. Continue?",
    );
    if (!confirmed) {
      return;
    }
    try {
      const identity = await persistCurrentIdentity();
      queryClient.setQueryData(["identity"], identity);
    } catch (error) {
      setPersistError(
        error instanceof Error
          ? error.message
          : "Failed to create a new identity. Please try again.",
      );
    }
  }, [queryClient]);

  const handleKeyImportBack = React.useCallback(() => {
    if (keyImportStage === "backup-password") {
      setKeyImportFormKey((current) => current + 1);
      setKeyImportStage("key-entry");
      return;
    }
    if (identityLost) {
      void handleLostModeBack();
      return;
    }
    showProfilePage();
  }, [handleLostModeBack, identityLost, keyImportStage, showProfilePage]);

  const chromeBackAction =
    currentPage === "profile"
      ? {
          disabled: profileStepState.isSaving,
          onClick: () => {
            setMembershipError(null);
            setIsCommunityChangeOpen(true);
          },
        }
      : currentPage === "key-import"
        ? {
            label:
              keyImportStage === "backup-password"
                ? "Back"
                : identityLost
                  ? "Start new identity"
                  : "Back",
            disabled: isKeyImporting,
            onClick: handleKeyImportBack,
          }
        : currentPage === "avatar"
          ? {
              disabled:
                avatarStepState.isSaving || avatarStepState.isUploadingAvatar,
              onClick: showProfilePage,
            }
          : undefined;

  if (currentPage === "membership-denied") {
    return (
      <>
        <MembershipDenied
          activeRelayUrl={activeCommunity?.relayUrl ?? ""}
          onBack={() => {
            setTransitionDirection("backward");
            setCurrentPage(deniedFromPage);
          }}
          onChangeCommunity={() => setIsCommunityChangeOpen(true)}
          onContinueWithoutProfile={skipForNow}
          onImportKey={importExistingKey}
          onRetry={() => {
            void saveProfileAndContinue(membershipRetryPage);
          }}
          pubkey={deniedPubkey}
        />
        {isCommunityChangeOpen ? (
          <CommunityChangeOverlay
            onClose={() => setIsCommunityChangeOpen(false)}
          />
        ) : null}
      </>
    );
  }

  return (
    <>
      <div
        className="buzz-onboarding-neutral-theme buzz-startup-shell flex items-start justify-center overflow-y-auto bg-background px-4 pb-28 pt-[106px] text-foreground"
        data-testid="onboarding-gate"
        data-system-color-scheme={systemColorScheme}
      >
        <StartupWindowDragRegion />
        <OnboardingChrome current={currentStep} total={totalOnboardingSteps} />
        <OnboardingFooterProvider backAction={chromeBackAction}>
          <div
            className={`relative flex w-full flex-col items-center text-center ${
              currentPage === "avatar" ? "max-w-[1080px]" : "max-w-[500px]"
            }`}
          >
            {membershipError &&
            (currentPage === "profile" || currentPage === "avatar") ? (
              <div className="mb-4 w-full max-w-[500px] rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm">
                {membershipError.kind === "unreachable" ? (
                  <>
                    <p className="font-medium text-destructive">
                      Can't reach this relay
                    </p>
                    <p className="mt-1 text-muted-foreground">
                      Check your connection or change your community.
                    </p>
                    <Button
                      className="mt-3"
                      onClick={() => setIsCommunityChangeOpen(true)}
                      size="sm"
                      variant="outline"
                    >
                      Change community
                    </Button>
                  </>
                ) : (
                  <>
                    <p className="font-medium text-destructive">
                      {membershipError.message ?? "Something went wrong"}
                    </p>
                    <p className="mt-1 text-muted-foreground">
                      The relay returned an error. Try again.
                    </p>
                  </>
                )}
              </div>
            ) : null}

            {currentPage === "profile" ? (
              <ProfileStep
                actions={{
                  advanceWithoutSaving: advanceFromProfileWithoutSaving,
                  clearAvatarDraft: resetAvatarDraft,
                  importExistingKey: showKeyImportPage,
                  onUploadingChange: setIsUploadingAvatar,
                  skipForNow,
                  submit: () => {
                    void saveProfileAndContinue("avatar");
                  },
                  updateAvatarUrl: updateAvatarUrlDraft,
                  updateDisplayName: updateDisplayNameDraft,
                }}
                direction={transitionDirection}
                state={profileStepState}
                usesExistingIdentity
              />
            ) : currentPage === "key-import" ? (
              <OnboardingSlideTransition
                className="flex w-full flex-col items-center text-center"
                direction={transitionDirection}
                transitionKey={`key-import-${transitionDirection}`}
              >
                <div className="w-full max-w-[440px]">
                  {identityLost ? (
                    <>
                      <h1 className="text-title font-normal text-foreground">
                        Re-import your key
                      </h1>
                      <p className="mt-5 text-sm leading-6 text-muted-foreground">
                        Your identity is no longer in the system keyring.
                        Re-import your nsec to restore it — Buzz will restart to
                        finish recovery. Or go back to start a new identity with
                        a fresh key.
                      </p>
                    </>
                  ) : (
                    <>
                      <h1 className="text-title font-normal text-foreground">
                        Use your existing key
                      </h1>
                      <p className="mt-5 text-sm leading-6 text-muted-foreground">
                        Import your Nostr private key to use that identity with
                        Buzz. If this key already has a profile on the relay,
                        your name and avatar are restored automatically.
                      </p>
                    </>
                  )}
                </div>

                {persistError ? (
                  <p className="mt-4 w-full max-w-[440px] text-sm text-destructive">
                    {persistError}
                  </p>
                ) : null}

                <NostrKeyImportForm
                  key={keyImportFormKey}
                  onBack={handleKeyImportBack}
                  onImport={importExistingKey}
                  onImportingChange={setIsKeyImporting}
                  onStageChange={setKeyImportStage}
                  showBack={false}
                  showPasswordStageBack={false}
                />
              </OnboardingSlideTransition>
            ) : (
              <AvatarStep
                actions={{
                  advanceWithoutSaving: complete,
                  back: showProfilePage,
                  onUploadingChange: setIsUploadingAvatar,
                  skipForNow,
                  submit: () => {
                    void saveProfileAndContinue("complete");
                  },
                  updateAvatarUrl: updateAvatarUrlDraft,
                }}
                direction={transitionDirection}
                showAlwaysSkip={true}
                showBack={false}
                state={avatarStepState}
              />
            )}
          </div>
        </OnboardingFooterProvider>
      </div>
      {isCommunityChangeOpen ? (
        <CommunityChangeOverlay
          onClose={() => setIsCommunityChangeOpen(false)}
        />
      ) : null}
    </>
  );
}
