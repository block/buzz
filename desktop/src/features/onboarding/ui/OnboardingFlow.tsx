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
import { StartupWindowDragRegion } from "@/shared/ui/StartupWindowDragRegion";
import { StepProgress } from "@/shared/ui/step-progress";
import { AvatarStep } from "./AvatarStep";
import { MembershipDenied } from "./MembershipDenied";
import { NostrKeyImportForm } from "./NostrKeyImportForm";
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

/**
 * Check whether the relay denies access due to membership gating.
 *
 * Uses the standard relay message path to read the NIP-43 membership snapshot.
 *
 * Returns `true` if denied, `false` if the user is a member (or if the
 * relay doesn't enforce membership / isn't reachable).
 */
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

async function checkMembershipDenied(): Promise<boolean> {
  try {
    const { membership, snapshotFound } = await getMyRelayMembershipLookup();
    return snapshotFound && membership === null;
  } catch (error) {
    if (isRelayMembershipDeniedError(error)) {
      return true;
    }
    // Network errors, 401s, 500s — not membership denials.
    return false;
  }
}

type OnboardingFlowProps = {
  actions: OnboardingActions;
  canBackToWorkspaceSetup: boolean;
  identityLost?: boolean;
  initialProfile: OnboardingProfileSeed;
  onBackToWorkspaceSetup: () => void;
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
  canBackToWorkspaceSetup,
  identityLost = false,
  initialProfile,
  onBackToWorkspaceSetup,
}: OnboardingFlowProps) {
  const { complete, skipForNow } = actions;
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
  const [membershipRetryPage, setMembershipRetryPage] =
    React.useState<OnboardingPage>("avatar");
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
    if (currentPage === "profile" || currentPage === "avatar") {
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
    setTransitionDirection("backward");
    setCurrentPage("profile");
  }, []);

  const showKeyImportPage = React.useCallback(() => {
    setTransitionDirection("forward");
    setCurrentPage("key-import");
  }, []);

  const saveProfileAndContinue = React.useCallback(
    async (nextPage: OnboardingPage) => {
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
        const denied = await checkMembershipDenied();
        if (denied) {
          try {
            const identity = await getIdentity();
            setDeniedPubkey(identity.pubkey);
          } catch {
            setDeniedPubkey("");
          }
          setMembershipRetryPage(nextPage);
          setCurrentPage("membership-denied");
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
              setMembershipRetryPage(nextPage);
              setCurrentPage("membership-denied");
              return;
            }

            // Error falls through to the error banner / recovery buttons.
            return;
          }
        }

        if (nextPage === "avatar") {
          showAvatarPage();
          return;
        }

        if (nextPage === "theme") {
          showThemePage();
          return;
        }

        showSetupPage();
      } finally {
        setIsProfileAdvancePending(false);
      }
    },
    [
      isProfileAdvancePending,
      profileDraft,
      profileUpdateMutation,
      savedProfile,
      showAvatarPage,
      showSetupPage,
      showThemePage,
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
  const currentStep =
    currentPage === "profile" || currentPage === "key-import"
      ? 2
      : currentPage === "avatar"
        ? 3
        : currentPage === "theme"
          ? 4
          : 5;
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
      <MembershipDenied
        onChangeKey={
          canBackToWorkspaceSetup
            ? () => {
                setTransitionDirection("backward");
                onBackToWorkspaceSetup();
              }
            : undefined
        }
        onImportKey={canBackToWorkspaceSetup ? undefined : importExistingKey}
        onRetry={() => {
          void saveProfileAndContinue(membershipRetryPage);
        }}
        pubkey={deniedPubkey}
      />
    );
  }

  return (
    <div
      className={`buzz-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground ${
        currentPage === "profile" ||
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
        className={`relative flex w-full flex-col items-center text-center ${
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
          />
        </OnboardingSlideTransition>

        {currentPage === "profile" ? (
          <ProfileStep
            actions={{
              advanceWithoutSaving: advanceFromProfileWithoutSaving,
              back: canBackToWorkspaceSetup
                ? () => {
                    setTransitionDirection("backward");
                    onBackToWorkspaceSetup();
                  }
                : undefined,
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
                    Your identity is no longer in the system keyring. Re-import
                    your nsec to restore it — Buzz will restart to finish
                    recovery. Or go back to start a new identity with a fresh
                    key.
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
        ) : currentPage === "avatar" ? (
          <AvatarStep
            actions={{
              advanceWithoutSaving: () => showThemePage(),
              back: showProfilePage,
              onUploadingChange: setIsUploadingAvatar,
              skipForNow,
              submit: () => {
                void saveProfileAndContinue("theme");
              },
              updateAvatarUrl: updateAvatarUrlDraft,
            }}
            direction={transitionDirection}
            state={avatarStepState}
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
  );
}
