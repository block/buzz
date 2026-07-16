import * as React from "react";
import { flushSync } from "react-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  profileQueryKey,
  useUpdateProfileMutation,
} from "@/features/profile/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { getMyRelayMembershipLookup } from "@/shared/api/relayMembers";
import {
  getIdentity,
  importIdentity,
  persistCurrentIdentity,
} from "@/shared/api/tauriIdentity";
import {
  ACCENT_STORAGE_KEY,
  NEUTRAL_ACCENT,
  THEME_STORAGE_KEY,
  useTheme,
} from "@/shared/theme/ThemeProvider";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { ONBOARDING_DEFAULT_THEME_NAME } from "@/shared/theme/theme-loader";
import { Button } from "@/shared/ui/button";
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { StepProgress } from "@/shared/ui/step-progress";
import { AvatarStep } from "./AvatarStep";
import { BackupStep } from "./BackupStep";
import { MembershipDenied } from "./MembershipDenied";
import { NostrKeyImportForm } from "./NostrKeyImportForm";
import { useCommunities } from "@/features/communities/useCommunities";
import { CommunityChangeOverlay } from "@/features/communities/ui/CommunityChangeOverlay";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import { ProfileStep } from "./ProfileStep";
import { SetupStep } from "./SetupStep";
import { ThemeStep, preloadThemePreviewVars } from "./ThemeStep";
import type {
  OnboardingActions,
  OnboardingPage,
  OnboardingProfileSeed,
  OnboardingProfileValues,
  ProfileStepState,
} from "./types";

function isRelayMembershipDeniedError(error: unknown): boolean {
  if (!(error instanceof Error)) {
    return false;
  }

  return (
    error.message.includes("You must be a relay member") ||
    error.message.includes("relay_membership_required") ||
    error.message.includes("restricted: not a relay member") ||
    error.message.includes("invalid: you are not a relay member")
  );
}

type MembershipCheckResult = "denied" | "ok" | "unreachable" | "error";

async function checkMembershipStatus(): Promise<MembershipCheckResult> {
  try {
    const { membership, snapshotFound } = await getMyRelayMembershipLookup();
    if (snapshotFound && membership === null) return "denied";
    return "ok";
  } catch (error) {
    if (isRelayMembershipDeniedError(error)) return "denied";
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
  const { accentColor, setAccentColor, setTheme, themeName } = useTheme();

  const ensureThemeStepDefaults = React.useCallback(() => {
    const hasStoredTheme =
      window.localStorage.getItem(THEME_STORAGE_KEY) !== null;
    const hasStoredAccent =
      window.localStorage.getItem(ACCENT_STORAGE_KEY) !== null;

    if (!hasStoredTheme && themeName !== ONBOARDING_DEFAULT_THEME_NAME) {
      setTheme(ONBOARDING_DEFAULT_THEME_NAME);
    }

    if (!hasStoredAccent && accentColor !== NEUTRAL_ACCENT) {
      setAccentColor(NEUTRAL_ACCENT);
    }
  }, [accentColor, setAccentColor, setTheme, themeName]);

  React.useEffect(() => {
    if (
      currentPage === "profile" ||
      currentPage === "backup" ||
      currentPage === "avatar"
    ) {
      void preloadThemePreviewVars().catch(() => undefined);
    }

    if (currentPage === "avatar") {
      ensureThemeStepDefaults();
    }
  }, [currentPage, ensureThemeStepDefaults]);

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

  const showSetupPage = React.useCallback(() => {
    setTransitionDirection("forward");
    setCurrentPage("setup");
  }, []);

  const showThemePage = React.useCallback(
    (direction: OnboardingTransitionDirection = "forward") => {
      ensureThemeStepDefaults();
      setTransitionDirection(direction);
      setCurrentPage("theme");
    },
    [ensureThemeStepDefaults],
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
          try {
            const identity = await getIdentity();
            setDeniedPubkey(identity.pubkey);
          } catch {
            setDeniedPubkey("");
          }
          setDeniedFromPage((prev) =>
            currentPage === "membership-denied" ? prev : currentPage,
          );
          setMembershipRetryPage(nextPage);
          setCurrentPage("membership-denied");
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
              try {
                const identity = await getIdentity();
                setDeniedPubkey(identity.pubkey);
              } catch {
                setDeniedPubkey("");
              }
              setDeniedFromPage((prev) =>
                currentPage === "membership-denied" ? prev : currentPage,
              );
              setMembershipRetryPage(nextPage);
              setCurrentPage("membership-denied");
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
      currentPage,
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
  const hideFixedProgressOnCompact =
    currentPage === "avatar" || currentPage === "theme";

  // Swapping the identity changes the pubkey, which remounts this flow
  // (keyed on pubkey in App.tsx) and re-runs the onboarding gate: the new
  // key's relay profile reseeds the steps, and a key that already finished
  // onboarding on this machine skips straight into the app.
  const importExistingKey = React.useCallback(
    async (nsec: string) => {
      const identity = await importIdentity(nsec);
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
        className={`buzz-startup-shell flex items-start justify-center overflow-y-auto bg-background px-4 py-8 text-foreground ${
          currentPage === "profile" ||
          currentPage === "backup" ||
          currentPage === "avatar" ||
          currentPage === "key-import"
            ? "buzz-onboarding-neutral-theme"
            : ""
        }`}
        data-testid="onboarding-gate"
        data-system-color-scheme={systemColorScheme}
      >
        <StartupWindowDragRegion />
        <div
          className={`relative my-auto flex w-full flex-col items-center text-center ${
            currentPage === "theme"
              ? "max-w-[1180px]"
              : currentPage === "avatar"
                ? "max-w-[1080px]"
                : currentPage === "setup"
                  ? "max-w-[920px]"
                  : "max-w-[500px]"
          }`}
        >
          <OnboardingSlideTransition
            className="w-auto"
            containerClassName={`fixed bottom-12 left-1/2 z-40 w-auto -translate-x-1/2 ${
              hideFixedProgressOnCompact ? "max-lg:hidden" : ""
            }`}
            direction={transitionDirection}
            transitionKey={`progress-${currentStep}-${transitionDirection}-${
              hideFixedProgressOnCompact ? "compact-hidden" : "visible"
            }`}
          >
            <StepProgress
              activeSegmentClassName="bg-primary"
              completeSegmentClassName="bg-primary/35"
              currentStep={currentStep}
              inactiveSegmentClassName="bg-muted-foreground/25"
              totalSteps={totalOnboardingSteps}
            />
          </OnboardingSlideTransition>

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
                back: () => {
                  setMembershipError(null);
                  setIsCommunityChangeOpen(true);
                },
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
                    <h1 className="text-3xl font-semibold tracking-tight">
                      Re-import your key
                    </h1>
                    <p className="mt-3 text-sm leading-6 text-muted-foreground">
                      Your identity is no longer in the system keyring.
                      Re-import your nsec to restore it — Buzz will restart to
                      finish recovery. Or go back to start a new identity with a
                      fresh key.
                    </p>
                  </>
                ) : (
                  <>
                    <h1 className="text-3xl font-semibold tracking-tight">
                      Use your existing key
                    </h1>
                    <p className="mt-3 text-sm leading-6 text-muted-foreground">
                      Import your Nostr private key to use that identity with
                      Buzz. If this key already has a profile on the relay, your
                      name and avatar are restored automatically.
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
                backLabel={identityLost ? "Start new identity" : undefined}
                onBack={identityLost ? handleLostModeBack : showProfilePage}
                onImport={importExistingKey}
              />
            </OnboardingSlideTransition>
          ) : currentPage === "backup" ? (
            <BackupStep
              currentStep={currentStep}
              direction={transitionDirection}
              onBack={showProfilePage}
              onNext={() => showAvatarPage()}
              totalSteps={totalOnboardingSteps}
            />
          ) : currentPage === "avatar" ? (
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
              currentStep={currentStep}
              direction={transitionDirection}
              showAlwaysSkip={true}
              state={avatarStepState}
              totalSteps={totalOnboardingSteps}
            />
          ) : currentPage === "theme" ? (
            <ThemeStep
              actions={{
                skip: showSetupPage,
                submit: showSetupPage,
              }}
              direction={transitionDirection}
            />
          ) : (
            <SetupStep
              actions={{
                back: () => showThemePage("backward"),
                complete,
              }}
              direction={transitionDirection}
            />
          )}
        </div>
      </div>
      {isCommunityChangeOpen ? (
        <CommunityChangeOverlay
          onClose={() => setIsCommunityChangeOpen(false)}
        />
      ) : null}
    </>
  );
}
