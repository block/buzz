import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  type AvatarMode,
  parseEmojiAvatarDataUrl,
  ProfileAvatarEditor,
} from "@/features/profile/ui/ProfileAvatarEditor";
import { Plus } from "lucide-react";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import { AnimatePresence, motion } from "motion/react";
import * as React from "react";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { ProfileStepActions, ProfileStepState } from "./types";

type AvatarStepProps = {
  actions: {
    advanceWithoutSaving: ProfileStepActions["advanceWithoutSaving"];
    back: () => void;
    onUploadingChange: ProfileStepActions["onUploadingChange"];
    skipForNow: ProfileStepActions["skipForNow"];
    submit: ProfileStepActions["submit"];
    updateAvatarUrl: ProfileStepActions["updateAvatarUrl"];
  };
  direction: OnboardingTransitionDirection;
  /** When true, a ghost "Skip for now" button is always visible (not just on error). */
  showAlwaysSkip?: boolean;
  state: Pick<
    ProfileStepState,
    "avatar" | "isSaving" | "isUploadingAvatar" | "name" | "saveRecovery"
  >;
};

function ErrorBanner({ message }: { message: string | null }) {
  if (!message) {
    return null;
  }

  return (
    <p className="mx-auto mt-4 w-full max-w-[576px] rounded-md border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
      {message}
    </p>
  );
}

const NEUTRAL_EMOJI_PICKER_THEME_VARS = {
  "--buzz-emoji-picker-rgb-background":
    "var(--buzz-onboarding-emoji-picker-background)",
  "--buzz-emoji-picker-rgb-color": "var(--buzz-onboarding-emoji-picker-color)",
  "--buzz-emoji-picker-rgb-input": "var(--buzz-onboarding-emoji-picker-input)",
} as React.CSSProperties;

const AVATAR_ACTIONS_MOTION_TRANSITION = {
  duration: 0.25,
  ease: "easeOut",
} as const;

const AVATAR_POSITION_MOTION_TRANSITION = {
  duration: 0.25,
  ease: "easeOut",
} as const;

function AvatarPreview({
  avatarSquishKey,
  avatarUrl,
  previewName,
}: {
  avatarSquishKey: number;
  avatarUrl: string;
  previewName: string;
}) {
  const emojiAvatar = parseEmojiAvatarDataUrl(avatarUrl);
  const hasAvatarUrl = avatarUrl.trim().length > 0;

  return (
    <div className="flex h-48 w-48 items-center justify-center">
      {emojiAvatar ? (
        <div
          aria-label={`${previewName} avatar`}
          className="relative flex h-full w-full shrink-0 items-center justify-center overflow-hidden rounded-full shadow-xs transition-colors duration-[250ms] ease-out"
          data-testid="onboarding-avatar-preview"
          role="img"
          style={{ backgroundColor: emojiAvatar.color }}
        >
          <span
            className={cn(
              "buzz-avatar-emoji-glyph flex h-full w-full items-center justify-center text-[6rem] leading-[6.25rem]",
              avatarSquishKey > 0 && "buzz-avatar-squish",
            )}
            data-testid="onboarding-avatar-preview-emoji"
            key={avatarSquishKey}
          >
            {emojiAvatar.emoji}
          </span>
        </div>
      ) : !hasAvatarUrl ? (
        <div
          aria-label="Add a display image"
          className="flex h-full w-full shrink-0 items-center justify-center rounded-full border-2 border-dashed border-border bg-background text-primary shadow-xs"
          data-testid="onboarding-avatar-preview"
          role="img"
        >
          <Plus className="h-14 w-14" aria-hidden="true" />
        </div>
      ) : (
        <ProfileAvatar
          avatarUrl={avatarUrl}
          className="h-full w-full rounded-full text-5xl"
          iconClassName="h-14 w-14"
          label={previewName}
          testId="onboarding-avatar-preview"
        />
      )}
    </div>
  );
}

function AvatarStepActions({
  canSubmit,
  hidden,
  isSaving,
  isUploadingAvatar,
  onBack,
  onContinueWithoutSaving,
  onSkipForNow,
  onSubmit,
  saveRecovery,
  showAlwaysSkip,
}: {
  canSubmit: boolean;
  hidden: boolean;
  isSaving: boolean;
  isUploadingAvatar: boolean;
  onBack: () => void;
  onContinueWithoutSaving: () => void;
  onSkipForNow: () => void;
  onSubmit: () => void;
  saveRecovery: ProfileStepState["saveRecovery"];
  showAlwaysSkip: boolean;
}) {
  const areNavigationActionsDisabled = isSaving || isUploadingAvatar;

  return (
    <OnboardingFooter>
      <AnimatePresence initial={false} mode="popLayout">
        {hidden ? null : (
          <motion.div
            className="flex w-full origin-center flex-col items-center gap-3"
            animate={{
              opacity: 1,
              scale: 1,
            }}
            exit={{
              opacity: 0,
              scale: 0.94,
            }}
            initial={{
              opacity: 0,
              scale: 0.94,
            }}
            transition={AVATAR_ACTIONS_MOTION_TRANSITION}
          >
            <Button
              className={ONBOARDING_PRIMARY_CTA_CLASS}
              data-testid="onboarding-next"
              disabled={!canSubmit}
              onClick={onSubmit}
              type="button"
            >
              {isSaving || isUploadingAvatar ? (
                <Spinner
                  aria-label={isSaving ? "Saving profile" : "Uploading avatar"}
                  className="h-4 w-4 border-2"
                />
              ) : (
                "Next"
              )}
            </Button>

            {saveRecovery.canSkipForNow ? (
              // Error-recovery path: exits onboarding entirely when there is no
              // saved display name to fall back on.
              <Button
                className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                data-testid="onboarding-skip"
                disabled={areNavigationActionsDisabled}
                onClick={onSkipForNow}
                type="button"
                variant="ghost"
              >
                Skip for now
              </Button>
            ) : showAlwaysSkip && !saveRecovery.errorMessage ? (
              // Normal path: advances to the theme step without saving an avatar.
              <Button
                className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                data-testid="onboarding-skip"
                disabled={areNavigationActionsDisabled}
                onClick={onContinueWithoutSaving}
                type="button"
                variant="ghost"
              >
                Skip for now
              </Button>
            ) : null}

            {saveRecovery.canAdvanceWithoutSaving ? (
              <Button
                className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
                data-testid="onboarding-next-without-saving"
                disabled={areNavigationActionsDisabled}
                onClick={onContinueWithoutSaving}
                type="button"
                variant="ghost"
              >
                Continue without saving
              </Button>
            ) : null}

            <Button
              className="h-10 w-full text-muted-foreground hover:text-accent-foreground"
              data-testid="onboarding-back"
              disabled={areNavigationActionsDisabled}
              onClick={onBack}
              type="button"
              variant="ghost"
            >
              Back
            </Button>
          </motion.div>
        )}
      </AnimatePresence>
    </OnboardingFooter>
  );
}

export function AvatarStep({
  actions,
  direction,
  showAlwaysSkip = false,
  state,
}: AvatarStepProps) {
  const {
    advanceWithoutSaving,
    back,
    onUploadingChange,
    skipForNow,
    submit,
    updateAvatarUrl,
  } = actions;
  const { avatar, isSaving, isUploadingAvatar, name, saveRecovery } = state;
  const [avatarSquishKey, setAvatarSquishKey] = React.useState(0);
  const [avatarEditorMode, setAvatarEditorMode] =
    React.useState<AvatarMode>("image");
  const [animatedPreviewEl, setAnimatedPreviewEl] =
    React.useState<HTMLDivElement | null>(null);
  const [isAnimatedPreviewActive, setIsAnimatedPreviewActive] =
    React.useState(false);
  const [animatedPreviewCaption, setAnimatedPreviewCaption] = React.useState<
    string | null
  >(null);
  const [pendingAnimatedAvatarUrl, setPendingAnimatedAvatarUrl] =
    React.useState<string | null>(null);
  const [isCustomColorPickerOpen, setIsCustomColorPickerOpen] =
    React.useState(false);
  const hasAvatarDraft = avatar.draftUrl.trim().length > 0;
  const canSubmit = hasAvatarDraft && !isSaving && !isUploadingAvatar;
  const isAutoAdvancingAnimatedAvatar = pendingAnimatedAvatarUrl !== null;
  const shouldHideActionsForAnimatedAvatar =
    avatarEditorMode === "animated" &&
    (!hasAvatarDraft || isAutoAdvancingAnimatedAvatar);
  const areActionsHidden =
    isCustomColorPickerOpen || shouldHideActionsForAnimatedAvatar;
  const previewName =
    name.draftValue.trim() || name.savedValue.trim() || "Your avatar";
  const animateEmojiAvatarChange = React.useCallback(() => {
    setAvatarSquishKey((key) => key + 1);
  }, []);
  const handleAnimatedAvatarApply = React.useCallback((url: string) => {
    setPendingAnimatedAvatarUrl(url);
  }, []);

  React.useEffect(() => {
    if (!pendingAnimatedAvatarUrl) {
      return;
    }
    if (avatar.draftUrl !== pendingAnimatedAvatarUrl) {
      return;
    }
    if (saveRecovery.errorMessage) {
      setPendingAnimatedAvatarUrl(null);
      return;
    }
    if (isSaving || isUploadingAvatar) {
      return;
    }

    submit();
  }, [
    avatar.draftUrl,
    isSaving,
    isUploadingAvatar,
    pendingAnimatedAvatarUrl,
    saveRecovery.errorMessage,
    submit,
  ]);

  return (
    <OnboardingSlideTransition
      // pb clears the always-docked footer: the emoji/color grid is the tallest
      // onboarding content and overflows on short windows, so the shell's own
      // bottom reserve isn't enough to scroll the last rows out from under the
      // fixed CTA group + scrim.
      className="flex w-full flex-col items-center pb-20"
      data-testid="onboarding-page-avatar"
      direction={direction}
      transitionKey={`avatar-${direction}`}
    >
      <motion.div
        className="grid w-full max-w-[1080px] items-start gap-12 lg:grid-cols-[minmax(300px,420px)_minmax(0,500px)] lg:gap-16"
        layout="position"
        layoutDependency={`${avatarEditorMode}-${isCustomColorPickerOpen}`}
        transition={AVATAR_POSITION_MOTION_TRANSITION}
      >
        <div className="flex w-full flex-col items-center text-center lg:items-start lg:text-left">
          <div className="w-full max-w-[500px]">
            <h1 className="text-title font-normal text-foreground">
              Next, add a display image
            </h1>
            <p className="mt-5 text-sm leading-6 text-muted-foreground">
              Choose an image or emoji as your avatar
            </p>
          </div>

          <div className="mt-12 grid justify-items-center gap-3 lg:justify-items-start">
            <div className="relative h-48 w-48">
              <div
                className="pointer-events-none absolute inset-0 z-10"
                data-testid="onboarding-avatar-animated-preview-slot"
                ref={setAnimatedPreviewEl}
              />
              {isAnimatedPreviewActive ? null : (
                <AvatarPreview
                  avatarSquishKey={avatarSquishKey}
                  avatarUrl={avatar.draftUrl}
                  previewName={previewName}
                />
              )}
            </div>

            <AnimatePresence initial={false}>
              {animatedPreviewCaption ? (
                <motion.p
                  animate={{ opacity: 1, y: 0 }}
                  className="w-48 text-center text-sm font-medium text-muted-foreground"
                  exit={{ opacity: 0, y: -4 }}
                  initial={{ opacity: 0, y: 4 }}
                  transition={AVATAR_ACTIONS_MOTION_TRANSITION}
                >
                  {animatedPreviewCaption}
                </motion.p>
              ) : null}
            </AnimatePresence>
          </div>
        </div>

        <motion.div
          className="w-full"
          layout="position"
          layoutDependency={`${avatarEditorMode}-${isCustomColorPickerOpen}`}
          transition={AVATAR_POSITION_MOTION_TRANSITION}
        >
          <ProfileAvatarEditor
            animatedPreviewContainer={animatedPreviewEl}
            avatarUrl={avatar.draftUrl}
            disabled={isSaving}
            emojiPickerTheme="auto"
            emojiPickerThemeVars={NEUTRAL_EMOJI_PICKER_THEME_VARS}
            onAnimatedAvatarApply={handleAnimatedAvatarApply}
            onAnimatedPreviewActiveChange={setIsAnimatedPreviewActive}
            onAnimatedPreviewCaptionChange={setAnimatedPreviewCaption}
            onCustomColorPickerOpenChange={setIsCustomColorPickerOpen}
            onEmojiAvatarChange={animateEmojiAvatarChange}
            onModeChange={setAvatarEditorMode}
            onUploadingChange={onUploadingChange}
            onUrlChange={updateAvatarUrl}
            previewName={previewName}
            testIdPrefix="onboarding-avatar"
          />

          {saveRecovery.errorMessage ? (
            <ErrorBanner message={saveRecovery.errorMessage} />
          ) : null}

          <AvatarStepActions
            canSubmit={canSubmit}
            hidden={areActionsHidden}
            isSaving={isSaving}
            isUploadingAvatar={isUploadingAvatar}
            onBack={back}
            onContinueWithoutSaving={advanceWithoutSaving}
            onSkipForNow={skipForNow}
            onSubmit={submit}
            saveRecovery={saveRecovery}
            showAlwaysSkip={showAlwaysSkip}
          />
        </motion.div>
      </motion.div>
    </OnboardingSlideTransition>
  );
}
