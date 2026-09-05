import { Plus } from "lucide-react";
import * as React from "react";

import { AgentCreationPreview } from "@/features/agents/ui/AgentCreationPreview";
import { useAvatarPresentation } from "@/features/profile/avatarPresentationStore";
import type { AnimatedAvatarRecordingProcessor } from "@/features/profile/ui/AnimatedAvatarCapture.types";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  emojiAvatarDataUrl,
  parseEmojiAvatarDataUrl,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { ProfileAvatarEditor } from "@/features/profile/ui/ProfileAvatarEditor";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/shared/ui/dialog";
import { ONBOARDING_PRIMARY_CTA_CLASS } from "./OnboardingChrome";
import { OnboardingFooter } from "./OnboardingFooter";
import { OnboardingPreviewInput } from "./OnboardingPreviewInput";
import { useOnboardingPreviewCardLayout } from "./OnboardingPreviewShell";
import { ONBOARDING_PREVIEW_CARD_INPUT_CLASS } from "./onboardingPreviewCardStyles";

const NEUTRAL_EMOJI_PICKER_THEME_VARS = {
  "--buzz-emoji-picker-rgb-background":
    "var(--buzz-onboarding-emoji-picker-background)",
  "--buzz-emoji-picker-rgb-color": "var(--buzz-onboarding-emoji-picker-color)",
  "--buzz-emoji-picker-rgb-input": "var(--buzz-onboarding-emoji-picker-input)",
} as React.CSSProperties;

const PREVIEW_URL_AVATAR = emojiAvatarDataUrl("🐝", "#D7D72E");

function blobAsDataUrl(blob: Blob) {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error ?? new Error("Read failed."));
    reader.onload = () => resolve(String(reader.result));
    reader.readAsDataURL(blob);
  });
}

const processPreviewImage = (file: File) => blobAsDataUrl(file);

const processPreviewAnimatedAvatar: AnimatedAvatarRecordingProcessor = ({
  posterBytes,
}) =>
  blobAsDataUrl(
    new Blob([Uint8Array.from(posterBytes).buffer], { type: "image/png" }),
  );

function AvatarCircle({
  avatarUrl,
  onClick,
  previewName,
  triggerRef,
}: {
  avatarUrl: string;
  onClick: () => void;
  previewName: string;
  triggerRef?: React.Ref<HTMLButtonElement>;
}) {
  const emojiAvatar = parseEmojiAvatarDataUrl(avatarUrl);
  const presentation = useAvatarPresentation(avatarUrl);
  const hasAvatar =
    avatarUrl.trim().length > 0 && presentation?.state !== "failed";

  return (
    <button
      aria-label={hasAvatar ? "Change your avatar" : "Add an avatar"}
      className="group block shrink-0 rounded-full"
      data-testid="community-avatar-open"
      onClick={onClick}
      ref={triggerRef}
      type="button"
    >
      {emojiAvatar ? (
        <span
          className="flex h-36 w-36 items-center justify-center overflow-hidden rounded-full text-5xl shadow-xs"
          style={{ backgroundColor: emojiAvatar.color }}
        >
          {emojiAvatar.emoji}
        </span>
      ) : hasAvatar ? (
        <ProfileAvatar
          avatarUrl={avatarUrl}
          className="h-36 w-36 rounded-full text-4xl"
          label={previewName}
          testId="community-avatar-circle"
        />
      ) : (
        <span
          className="flex h-36 w-36 items-center justify-center rounded-full bg-white/30 text-[var(--buzz-onboarding-backup-ink)] transition-colors group-hover:bg-white/40"
          data-testid="community-avatar-empty"
        >
          <Plus className="h-7 w-7" aria-hidden="true" />
        </span>
      )}
    </button>
  );
}

export function CommunityProfileStage({
  avatarUrl,
  displayName,
  errorMessage,
  isPending,
  isUploadingAvatar,
  onAvatarUrlChange,
  onDisplayNameChange,
  onNext,
  onUploadingChange,
  previewMode = false,
}: {
  avatarUrl: string;
  displayName: string;
  errorMessage?: string;
  isPending: boolean;
  isUploadingAvatar: boolean;
  onAvatarUrlChange: (value: string) => void;
  onDisplayNameChange: (value: string) => void;
  onNext: () => void;
  onUploadingChange: (isUploading: boolean) => void;
  previewMode?: boolean;
}) {
  const cardLayout = useOnboardingPreviewCardLayout();
  const [localAvatarPreviewUrl, setLocalAvatarPreviewUrl] = React.useState<
    string | null
  >(null);
  const [avatarSquishKey, setAvatarSquishKey] = React.useState(0);
  const [isAvatarEditorOpen, setIsAvatarEditorOpen] = React.useState(false);
  const [animatedPreviewEl, setAnimatedPreviewEl] =
    React.useState<HTMLDivElement | null>(null);
  const [avatarModeTabsEl, setAvatarModeTabsEl] =
    React.useState<HTMLDivElement | null>(null);
  const [isAnimatedPreviewActive, setIsAnimatedPreviewActive] =
    React.useState(false);
  const [animatedPreviewCaption, setAnimatedPreviewCaption] = React.useState<
    string | null
  >(null);
  const nameInputRef = React.useRef<HTMLInputElement | null>(null);
  const avatarTriggerRef = React.useRef<HTMLButtonElement | null>(null);
  const avatarEditorContentRef = React.useRef<HTMLDivElement | null>(null);
  const [avatarEditorDialogHeight, setAvatarEditorDialogHeight] =
    React.useState<number | null>(null);
  const animateEmojiAvatarChange = React.useCallback(() => {
    setAvatarSquishKey((key) => key + 1);
  }, []);
  const updateAvatarUrl = React.useCallback(
    (nextAvatarUrl: string) => {
      if (
        !previewMode ||
        nextAvatarUrl.length === 0 ||
        nextAvatarUrl.startsWith("blob:") ||
        nextAvatarUrl.startsWith("data:image/")
      ) {
        onAvatarUrlChange(nextAvatarUrl);
        return;
      }
      onAvatarUrlChange(PREVIEW_URL_AVATAR);
    },
    [onAvatarUrlChange, previewMode],
  );

  React.useLayoutEffect(() => {
    if (!isAvatarEditorOpen) nameInputRef.current?.focus();
  }, [isAvatarEditorOpen]);

  React.useLayoutEffect(() => {
    if (!isAvatarEditorOpen) {
      setAvatarEditorDialogHeight(null);
      return;
    }
    const content = avatarEditorContentRef.current;
    if (!content) return;

    const updateHeight = () => {
      setAvatarEditorDialogHeight(content.getBoundingClientRect().height + 64);
    };
    updateHeight();
    const resizeObserver = new ResizeObserver(updateHeight);
    resizeObserver.observe(content);
    return () => resizeObserver.disconnect();
  }, [isAvatarEditorOpen]);

  return (
    <>
      <div
        className={cn(
          "flex min-h-0 w-full flex-1 flex-col transition-[filter,opacity] duration-200 ease-out",
          isAvatarEditorOpen && "pointer-events-none opacity-45 blur-[3px]",
        )}
        data-testid="community-profile-main"
      >
        <div className={cn("shrink-0", cardLayout && "text-left")}>
          <h1 className="text-title font-normal">Build your profile</h1>
          {cardLayout ? null : (
            <p className="mx-auto mt-3 max-w-[380px] text-sm leading-6 text-foreground/80">
              Add a name and avatar. They’ll show up on your messages,
              reactions, and agent handoffs.
            </p>
          )}
        </div>
        <div
          className={cn(
            "flex min-h-0 w-full flex-1 flex-col",
            cardLayout
              ? "items-start justify-start pt-6"
              : "items-center justify-center pt-8",
          )}
        >
          {cardLayout ? (
            <div className="w-full" data-testid="community-avatar-section">
              <AgentCreationPreview
                align="start"
                allowAnimated
                assetLabel="profile image"
                avatarUrl={avatarUrl || null}
                disabled={isPending}
                label={displayName.trim() || "Your profile"}
                onClearAvatar={() => updateAvatarUrl("")}
                onSelectAvatar={updateAvatarUrl}
                onUploadPendingChange={onUploadingChange}
                presentation="onboarding"
                processAnimatedAvatar={
                  previewMode ? processPreviewAnimatedAvatar : undefined
                }
                processImage={previewMode ? processPreviewImage : undefined}
                testIdPrefix="community-avatar"
              />
            </div>
          ) : (
            <AvatarCircle
              avatarUrl={avatarUrl}
              onClick={() => setIsAvatarEditorOpen(true)}
              previewName={displayName.trim() || "Your profile"}
              triggerRef={avatarTriggerRef}
            />
          )}
          {!cardLayout && animatedPreviewCaption ? (
            <p className="mt-1 text-xs text-muted-foreground">
              {animatedPreviewCaption}
            </p>
          ) : null}
          <div
            className={cn(
              "block w-full text-left",
              cardLayout ? "mt-6 max-w-none" : "mt-7 max-w-[412px]",
            )}
          >
            <label
              className={cn(
                "mb-2 block text-sm text-foreground",
                !cardLayout && "pl-4",
              )}
              htmlFor="community-display-name"
            >
              Your username
            </label>
            <OnboardingPreviewInput
              aria-label="Community username"
              autoCapitalize="none"
              autoComplete="username"
              autoCorrect="off"
              className={
                cardLayout
                  ? ONBOARDING_PREVIEW_CARD_INPUT_CLASS
                  : "h-14 rounded-2xl border-[color:rgb(var(--buzz-onboarding-avatar-control-fg)_/_0.28)] bg-[rgb(var(--buzz-onboarding-avatar-dialog-bg)/0.95)] px-5 text-sm shadow-none placeholder:text-muted-foreground/60 focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-[color:rgb(var(--buzz-onboarding-avatar-control-fg)_/_0.5)] md:text-sm"
              }
              data-testid="community-profile-name-key"
              disabled={isPending || isUploadingAvatar}
              id="community-display-name"
              onChange={(event) => onDisplayNameChange(event.target.value)}
              placeholder="Enter your username here"
              ref={nameInputRef}
              smooth={cardLayout}
              spellCheck={false}
              type="text"
              value={displayName}
            />
          </div>
        </div>
        {errorMessage ? (
          <p className="mt-4 text-sm text-destructive">{errorMessage}</p>
        ) : null}
      </div>
      <OnboardingFooter
        className={cn(
          "transition-[filter,opacity] duration-200 ease-out",
          isAvatarEditorOpen && "pointer-events-none opacity-45 blur-[3px]",
        )}
      >
        <Button
          className={`${ONBOARDING_PRIMARY_CTA_CLASS} w-20`}
          data-testid="community-profile-next"
          disabled={!displayName.trim() || isPending || isUploadingAvatar}
          onClick={onNext}
          type="button"
        >
          Next
        </Button>
      </OnboardingFooter>
      <Dialog
        onOpenChange={(open) => setIsAvatarEditorOpen(open)}
        open={isAvatarEditorOpen}
      >
        <DialogContent
          className="buzz-onboarding-neutral-theme w-[min(calc(100vw-2rem),920px)] max-w-[920px] gap-0 overflow-hidden rounded-[18px] bg-[rgb(var(--buzz-onboarding-avatar-dialog-bg))] px-8 pb-6 pt-10 text-sm text-foreground shadow-[0_28px_90px_rgb(var(--buzz-onboarding-avatar-dialog-shadow)_/_0.28),0_8px_28px_rgb(var(--buzz-onboarding-avatar-dialog-shadow)_/_0.18)] transition-[height] duration-[250ms] ease-out"
          closeButtonClassName="right-6 top-6 h-10 w-10 rounded-full bg-[rgb(var(--buzz-onboarding-avatar-action-bg))] text-[rgb(var(--buzz-onboarding-avatar-action-fg))] hover:bg-[rgb(var(--buzz-onboarding-avatar-action-bg)/0.9)] hover:text-[rgb(var(--buzz-onboarding-avatar-action-fg))]"
          data-system-color-scheme="light"
          data-testid="community-avatar-editor-key-frame"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            avatarTriggerRef.current?.focus();
          }}
          overlayVariant="transparent"
          style={
            avatarEditorDialogHeight === null
              ? undefined
              : { height: avatarEditorDialogHeight }
          }
        >
          <DialogTitle className="sr-only">Edit your avatar</DialogTitle>
          <div
            className={cn(
              "grid items-center gap-8",
              cardLayout
                ? "grid-cols-[200px_minmax(0,1fr)] items-stretch"
                : "md:grid-cols-[240px_minmax(0,1fr)]",
            )}
            ref={avatarEditorContentRef}
          >
            <div
              className={cn(
                "flex flex-col items-center gap-3",
                cardLayout
                  ? "min-h-[454px] justify-start px-2 py-0"
                  : "min-h-[320px] justify-center px-6 py-8",
              )}
              data-testid="community-avatar-live-preview-panel"
            >
              <div
                className={cn(
                  "relative shrink-0",
                  cardLayout ? "h-40 w-40" : "h-48 w-48",
                )}
              >
                <div
                  className="pointer-events-none absolute inset-0 z-10"
                  data-testid="community-avatar-animated-preview-slot"
                  ref={setAnimatedPreviewEl}
                />
                {isAnimatedPreviewActive ? null : localAvatarPreviewUrl ? (
                  <ProfileAvatar
                    avatarUrl={localAvatarPreviewUrl}
                    className="h-full w-full rounded-full text-5xl"
                    label={displayName.trim() || "Your profile"}
                    testId="community-avatar-live-preview"
                  />
                ) : (
                  (() => {
                    const emojiAvatar = parseEmojiAvatarDataUrl(avatarUrl);
                    return emojiAvatar ? (
                      <div
                        aria-label={`${displayName.trim() || "Your profile"} avatar`}
                        className="flex h-full w-full items-center justify-center overflow-hidden rounded-full text-6xl shadow-xs"
                        data-testid="community-avatar-live-preview"
                        role="img"
                        style={{ backgroundColor: emojiAvatar.color }}
                      >
                        <span
                          className={cn(
                            avatarSquishKey > 0 && "buzz-avatar-squish",
                          )}
                          data-testid="community-avatar-live-preview-emoji"
                          key={avatarSquishKey}
                        >
                          {emojiAvatar.emoji}
                        </span>
                      </div>
                    ) : (
                      <ProfileAvatar
                        avatarUrl={avatarUrl || null}
                        className="h-full w-full rounded-full text-5xl"
                        label={displayName.trim() || "Your profile"}
                        testId="community-avatar-live-preview"
                      />
                    );
                  })()
                )}
              </div>
              {animatedPreviewCaption ? (
                <p className="text-center text-sm text-muted-foreground">
                  {animatedPreviewCaption}
                </p>
              ) : null}
              {cardLayout ? (
                <div
                  className="mt-3 w-full"
                  data-testid="community-avatar-mode-tabs-slot"
                  ref={setAvatarModeTabsEl}
                />
              ) : null}
            </div>
            <ProfileAvatarEditor
              animatedPreviewContainer={animatedPreviewEl}
              avatarUrl={avatarUrl}
              disabled={isPending}
              donePending={isUploadingAvatar}
              emojiPickerTheme="auto"
              emojiPickerThemeVars={NEUTRAL_EMOJI_PICKER_THEME_VARS}
              onAnimatedPreviewActiveChange={setIsAnimatedPreviewActive}
              onAnimatedPreviewCaptionChange={setAnimatedPreviewCaption}
              onDone={() => setIsAvatarEditorOpen(false)}
              onEmojiAvatarChange={animateEmojiAvatarChange}
              onLocalPreviewChange={setLocalAvatarPreviewUrl}
              onUploadingChange={onUploadingChange}
              onUrlChange={updateAvatarUrl}
              modeTabsContainer={cardLayout ? avatarModeTabsEl : undefined}
              modeTabsOrientation={cardLayout ? "vertical" : "horizontal"}
              presentation="onboarding-modal"
              previewName={displayName.trim() || "Your profile"}
              processAnimatedAvatar={
                previewMode ? processPreviewAnimatedAvatar : undefined
              }
              processImage={previewMode ? processPreviewImage : undefined}
              testIdPrefix="community-avatar"
            />
          </div>
        </DialogContent>
      </Dialog>
    </>
  );
}
