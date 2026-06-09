import type { AcpRuntime, Profile } from "@/shared/api/types";

export type OnboardingPage = "profile" | "setup" | "membership-denied";

export type OnboardingActions = {
  complete: () => void;
  skipForNow: () => void;
};

export type OnboardingProfileSeed = {
  profile?: Profile;
};

export type OnboardingProfileValues = {
  avatarUrl: string;
  displayName: string;
};

export type ProfileStepSaveRecovery = {
  canAdvanceWithoutSaving: boolean;
  canSkipForNow: boolean;
  errorMessage: string | null;
};

export type ProfileStepNameState = {
  draftValue: string;
  savedValue: string;
};

export type ProfileStepAvatarState = {
  draftUrl: string;
  savedUrl: string;
};

export type ProfileStepState = {
  avatar: ProfileStepAvatarState;
  /** Bech32-encoded current pubkey (npub1…), shown so the user can confirm
   *  which identity they're saving the profile for. */
  currentNpub: string | null;
  isUploadingAvatar: boolean;
  isSaving: boolean;
  name: ProfileStepNameState;
  saveRecovery: ProfileStepSaveRecovery;
};

export type ProfileStepActions = {
  advanceWithoutSaving: () => void;
  clearAvatarDraft: () => void;
  importIdentity: (nsec: string) => Promise<void>;
  onUploadingChange: (isUploading: boolean) => void;
  skipForNow: () => void;
  submit: () => void;
  updateAvatarUrl: (value: string) => void;
  updateDisplayName: (value: string) => void;
};

export type SetupStepActions = {
  back: () => void;
  complete: () => void;
};

export type SetupStepRuntimeState = {
  errorMessage: string | null;
  isChecking: boolean;
  items: AcpRuntime[];
  showSetupLaterHint: boolean;
};

export type SetupStepState = {
  runtimeProviders: SetupStepRuntimeState;
};
