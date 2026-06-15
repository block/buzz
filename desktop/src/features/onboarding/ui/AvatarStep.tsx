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
import { StepProgress } from "@/shared/ui/step-progress";
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
              "buzz-avatar-emoji-glyph flex h-full w-full items-center justify-center text-[6rem] leading-[100px]",
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
}) {
  return (
    <AnimatePresence initial={false} mode="popLayout">
      {hidden ? null : (
        <motion.div
          className="mx-auto mt-4 flex w-full max-w-[576px] origin-center flex-col gap-3 max-lg:pointer-events-none max-lg:fixed max-lg:inset-x-0 max-lg:bottom-0 max-lg:z-40 max-lg:mt-0 max-lg:max-w-none max-lg:border-t max-lg:border-border max-lg:bg-background max-lg:p-4 max-lg:pb-[max(1rem,env(safe-area-inset-bottom))]"
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
            className="h-10 w-full max-lg:pointer-events-auto"
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
            <Button
              className="h-10 w-full text-muted-foreground hover:text-accent-foreground max-lg:pointer-events-auto"
              data-testid="onboarding-skip"
              disabled={isSaving}
              onClick={onSkipForNow}
              type="button"
              variant="ghost"
            >
              Skip for now
            </Button>
          ) : null}

          {saveRecovery.canAdvanceWithoutSaving ? (
            <Button
              className="h-10 w-full text-muted-foreground hover:text-accent-foreground max-lg:pointer-events-auto"
              data-testid="onboarding-next-without-saving"
              disabled={isSaving}
              onClick={onContinueWithoutSaving}
              type="button"
              variant="ghost"
            >
              Continue without saving
            </Button>
          ) : null}

          <Button
            className="h-10 w-full text-muted-foreground hover:text-accent-foreground max-lg:pointer-events-auto"
            data-testid="onboarding-back"
            disabled={isSaving}
            onClick={onBack}
            type="button"
            variant="ghost"
          >
            Back
          </Button>

          <StepProgress
            activeSegmentClassName="bg-primary"
            className="mt-1 max-lg:pointer-events-auto lg:hidden"
            completeSegmentClassName="bg-primary/35"
            currentStep={3}
            inactiveSegmentClassName="bg-muted-foreground/25"
          />
        </motion.div>
      )}
    </AnimatePresence>
  );
}

export function AvatarStep({ actions, direction, state }: AvatarStepProps) {
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
  const [isCustomColorPickerOpen, setIsCustomColorPickerOpen] =
    React.useState(false);
  const canSubmit =
    avatar.draftUrl.trim().length > 0 && !isSaving && !isUploadingAvatar;
  const previewName =
    name.draftValue.trim() || name.savedValue.trim() || "Your avatar";
  const animateEmojiAvatarChange = React.useCallback(() => {
    setAvatarSquishKey((key) => key + 1);
  }, []);

  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center pb-60 lg:pb-0"
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
            <h1 className="text-3xl font-semibold text-foreground">
              Next, add a display image
            </h1>
            <p className="mt-3 text-sm leading-6 text-muted-foreground">
              Choose an image or emoji as your avatar
            </p>
          </div>

          <div className="mt-12">
            <AvatarPreview
              avatarSquishKey={avatarSquishKey}
              avatarUrl={avatar.draftUrl}
              previewName={previewName}
            />
          </div>
        </div>

        <motion.div
          className="w-full"
          layout="position"
          layoutDependency={`${avatarEditorMode}-${isCustomColorPickerOpen}`}
          transition={AVATAR_POSITION_MOTION_TRANSITION}
        >
          <ProfileAvatarEditor
            avatarUrl={avatar.draftUrl}
            disabled={isSaving}
            emojiPickerTheme="auto"
            emojiPickerThemeVars={NEUTRAL_EMOJI_PICKER_THEME_VARS}
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
            hidden={isCustomColorPickerOpen}
            isSaving={isSaving}
            isUploadingAvatar={isUploadingAvatar}
            onBack={back}
            onContinueWithoutSaving={advanceWithoutSaving}
            onSkipForNow={skipForNow}
            onSubmit={submit}
            saveRecovery={saveRecovery}
          />
        </motion.div>
      </motion.div>
    </OnboardingSlideTransition>
  );
}
