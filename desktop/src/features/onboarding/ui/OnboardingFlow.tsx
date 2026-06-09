import * as React from "react";
import { flushSync } from "react-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  profileQueryKey,
  useUpdateProfileMutation,
} from "@/features/profile/hooks";
import { relayClient } from "@/shared/api/relayClient";
import { getMyRelayMembershipLookup } from "@/shared/api/relayMembers";
import { getIdentity, importIdentity } from "@/shared/api/tauri";
import {
  ACCENT_STORAGE_KEY,
  NEUTRAL_ACCENT,
  THEME_STORAGE_KEY,
  useTheme,
} from "@/shared/theme/ThemeProvider";
import { useSystemColorScheme } from "@/shared/theme/useSystemColorScheme";
import { ONBOARDING_DEFAULT_THEME_NAME } from "@/shared/theme/theme-loader";
import { StepProgress } from "@/shared/ui/step-progress";
import { AvatarStep } from "./AvatarStep";
import { MembershipDenied } from "./MembershipDenied";
import type { OnboardingTransitionDirection } from "./OnboardingSlideTransition";
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
  initialProfile,
  onBackToWorkspaceSetup,
}: OnboardingFlowProps) {
  const { complete, skipForNow } = actions;
  const queryClient = useQueryClient();
  const savedProfile = resolveSavedProfile(initialProfile);
  const profileUpdateMutation = useUpdateProfileMutation();
  const { error: profileSaveError, isPending: isSavingProfile } =
    profileUpdateMutation;
  const [currentPage, setCurrentPage] =
    React.useState<OnboardingPage>("profile");
  const [profileDraft, setProfileDraft] =
    React.useState<OnboardingProfileValues>(savedProfile);
  const [deniedPubkey, setDeniedPubkey] = React.useState<string>("");
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

  const importDeniedKey = React.useCallback(
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
        onImportKey={canBackToWorkspaceSetup ? undefined : importDeniedKey}
        onRetry={() => {
          void saveProfileAndContinue(membershipRetryPage);
        }}
        pubkey={deniedPubkey}
      />
    );
  }

  return (
    <div
      className={`sprout-startup-shell flex items-center justify-center bg-background px-4 py-8 text-foreground ${
        currentPage === "profile" || currentPage === "avatar"
          ? "sprout-onboarding-neutral-theme"
          : ""
      }`}
      data-testid="onboarding-gate"
      data-system-color-scheme={systemColorScheme}
    >
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
        <StepProgress
          activeSegmentClassName="bg-primary"
          className={`fixed bottom-12 left-1/2 z-40 -translate-x-1/2 ${
            currentPage === "avatar" || currentPage === "theme"
              ? "max-lg:hidden"
              : ""
          }`}
          completeSegmentClassName="bg-primary/35"
          currentStep={
            currentPage === "profile"
              ? 2
              : currentPage === "avatar"
                ? 3
                : currentPage === "theme"
                  ? 4
                  : 5
          }
          inactiveSegmentClassName="bg-muted-foreground/25"
        />

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
