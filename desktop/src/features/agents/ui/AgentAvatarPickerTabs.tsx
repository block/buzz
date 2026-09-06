import emojiData from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import { Link2, UploadCloud } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import type * as React from "react";

import { AvatarCustomColorPanel } from "@/features/profile/ui/AvatarCustomColorPanel";
import { AnimatedAvatarCapture } from "@/features/profile/ui/AnimatedAvatarCapture";
import type { AnimatedAvatarRecordingProcessor } from "@/features/profile/ui/AnimatedAvatarCapture.types";
import {
  AVATAR_COLORS,
  AVATAR_COLOR_SWATCHES,
  CUSTOM_AVATAR_COLOR_SWATCH,
  EMOJI_MART_CATEGORIES,
  type AvatarColorSwatch,
  contrastColorForBackground,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import {
  AVATAR_APPLY_MOTION_TRANSITION,
  type EmojiMartEmoji,
} from "./AgentCreationPreview.utils";

function RemoveAvatarButton({
  actionButtonClassName,
  assetLabel,
  disabled,
  onClick,
}: {
  actionButtonClassName?: string;
  assetLabel: string;
  disabled: boolean;
  onClick: () => void;
}) {
  if (actionButtonClassName) {
    return (
      <Button
        className={actionButtonClassName}
        disabled={disabled}
        onClick={onClick}
        type="button"
      >
        Remove {assetLabel}
      </Button>
    );
  }

  return (
    <button
      className="flex min-h-8 w-full items-center justify-center rounded-lg text-xs text-destructive outline-hidden transition-colors duration-150 ease-out hover:bg-destructive/10 focus-visible:bg-destructive/10 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50 motion-reduce:transition-none"
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      Remove {assetLabel}
    </button>
  );
}

export function AgentAvatarImageTab({
  actionButtonClassName,
  assetLabel,
  avatarUrlDraft,
  disabled,
  hasAvatar,
  isUploading,
  onApplyUrl,
  onAvatarUrlDraftChange,
  onClearAvatar,
  onOpenUploadPicker,
  uploadErrorMessage,
}: {
  actionButtonClassName?: string;
  assetLabel: string;
  avatarUrlDraft: string;
  disabled: boolean;
  hasAvatar: boolean;
  isUploading: boolean;
  onApplyUrl: () => void;
  onAvatarUrlDraftChange: (value: string) => void;
  onClearAvatar?: () => void;
  onOpenUploadPicker: () => void;
  uploadErrorMessage: string | null;
}) {
  const shouldReduceMotion = useReducedMotion();
  const applyButtonTransition = shouldReduceMotion
    ? { duration: 0 }
    : AVATAR_APPLY_MOTION_TRANSITION;

  return (
    <div className="grid gap-2.5">
      <button
        className="relative flex h-[80px] flex-col items-center justify-center gap-1.5 overflow-hidden rounded-lg border border-transparent bg-muted text-foreground transition-[background-color,border-color,box-shadow,color] duration-200 ease-out hover:bg-muted/80 disabled:opacity-60 motion-reduce:transition-none"
        disabled={disabled || isUploading}
        onClick={onOpenUploadPicker}
        type="button"
      >
        {isUploading ? (
          <Spinner
            aria-hidden
            className="h-5 w-5 border-2 text-muted-foreground"
          />
        ) : (
          <UploadCloud className="h-5 w-5 text-muted-foreground" />
        )}
        <span className="text-xs font-medium text-muted-foreground">
          {isUploading ? "Uploading..." : "Drop or browse"}
        </span>
      </button>

      <div className="flex h-10 items-center gap-2.5 rounded-lg bg-muted px-3">
        <Link2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
        <input
          autoCapitalize="none"
          autoCorrect="off"
          className="min-w-0 flex-1 bg-transparent text-xs font-medium text-foreground outline-none placeholder:text-muted-foreground/50"
          disabled={disabled || isUploading}
          onChange={(event) => onAvatarUrlDraftChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onApplyUrl();
            }
          }}
          placeholder="Paste a URL"
          spellCheck={false}
          type="url"
          value={avatarUrlDraft}
        />
        <AnimatePresence initial={false}>
          {avatarUrlDraft.trim().length > 0 ? (
            <motion.div
              animate={{ opacity: 1, scale: 1, width: "auto" }}
              className="overflow-hidden"
              exit={{ opacity: 0, scale: 0.96, width: 0 }}
              initial={{ opacity: 0, scale: 0.96, width: 0 }}
              key="apply-url"
              transition={applyButtonTransition}
            >
              <Button
                className="h-6 px-2 text-2xs"
                disabled={disabled || isUploading}
                onClick={onApplyUrl}
                size="xs"
                type="button"
              >
                Apply
              </Button>
            </motion.div>
          ) : null}
        </AnimatePresence>
      </div>

      {uploadErrorMessage ? (
        <p className="rounded-lg bg-destructive/10 px-3 py-2 text-xs font-medium text-destructive">
          {uploadErrorMessage}
        </p>
      ) : null}

      {hasAvatar && onClearAvatar ? (
        <RemoveAvatarButton
          actionButtonClassName={actionButtonClassName}
          assetLabel={assetLabel}
          disabled={disabled || isUploading}
          onClick={onClearAvatar}
        />
      ) : null}
    </div>
  );
}

export function AgentAvatarEmojiTab({
  actionButtonClassName,
  assetLabel,
  colorDraft,
  customHue,
  customSaturation,
  customValue,
  disabled,
  emojiMartThemeVars,
  emojiPickerTheme = "auto",
  emojiPickerContainerRef,
  emojiSearchControlHeight,
  hasAvatar,
  isCustomColorPickerVisible,
  onClearAvatar,
  onColorSelect,
  onCommitCustomColor,
  onEmojiSelect,
  onHueChange,
  onSaturationValueChange,
  selectedColor,
  selectedEmoji,
  showCategoryNavigation = true,
  showSkinTonePicker = false,
  testIdPrefix,
}: {
  actionButtonClassName?: string;
  assetLabel: string;
  colorDraft: string;
  customHue: number;
  customSaturation: number;
  customValue: number;
  disabled: boolean;
  emojiMartThemeVars: React.CSSProperties;
  emojiPickerTheme?: "auto" | "light";
  emojiPickerContainerRef: React.RefObject<HTMLDivElement | null>;
  emojiSearchControlHeight?: string;
  hasAvatar: boolean;
  isCustomColorPickerVisible: boolean;
  onClearAvatar?: () => void;
  onColorSelect: (swatch: AvatarColorSwatch) => void;
  onCommitCustomColor: () => void;
  onEmojiSelect: (emoji: EmojiMartEmoji, event?: MouseEvent) => void;
  onHueChange: (value: number) => void;
  onSaturationValueChange: (saturation: number, value: number) => void;
  selectedColor: string;
  selectedEmoji: string | null;
  showCategoryNavigation?: boolean;
  showSkinTonePicker?: boolean;
  testIdPrefix: string;
}) {
  return (
    <div className="relative grid content-start gap-3">
      <div
        className="buzz-emoji-mart relative z-0 h-[280px] overflow-hidden rounded-lg bg-muted"
        ref={emojiPickerContainerRef}
        style={
          {
            ...emojiMartThemeVars,
            ...(emojiSearchControlHeight
              ? {
                  "--buzz-emoji-picker-search-control-height":
                    emojiSearchControlHeight,
                }
              : null),
          } as React.CSSProperties
        }
      >
        <Picker
          autoFocus
          categories={EMOJI_MART_CATEGORIES}
          data={emojiData}
          dynamicWidth
          emojiButtonRadius="999px"
          emojiButtonSize={44}
          emojiSize={32}
          icons="outline"
          navPosition={showCategoryNavigation ? "bottom" : "none"}
          onEmojiSelect={onEmojiSelect}
          previewPosition="none"
          searchPosition="sticky"
          set="native"
          skinTonePosition={showSkinTonePicker ? "search" : "none"}
          theme={emojiPickerTheme}
        />
      </div>

      <div className="grid grid-cols-12 justify-items-center gap-1.5 rounded-lg bg-muted p-3">
        {AVATAR_COLOR_SWATCHES.map((swatch) => {
          const isCustomSwatch = swatch === CUSTOM_AVATAR_COLOR_SWATCH;
          const isSelected = isCustomSwatch
            ? !AVATAR_COLORS.some(
                (color) => color.toUpperCase() === selectedColor.toUpperCase(),
              )
            : swatch.toUpperCase() === selectedColor.toUpperCase();

          return (
            <button
              aria-label={
                isCustomSwatch
                  ? selectedEmoji
                    ? "Choose custom color"
                    : "Choose an emoji first"
                  : `Use ${swatch} background`
              }
              aria-pressed={isSelected}
              className={cn(
                "relative h-6 w-6 rounded-full border border-border transition-transform duration-150 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring motion-reduce:transition-none",
                isCustomSwatch &&
                  !selectedEmoji &&
                  "cursor-not-allowed opacity-45 hover:scale-100 focus-visible:scale-100",
              )}
              disabled={isCustomSwatch && !selectedEmoji}
              key={swatch}
              onClick={() => onColorSelect(swatch)}
              style={{
                background: isCustomSwatch
                  ? isSelected
                    ? selectedColor
                    : "conic-gradient(from 0deg, #ff4d4d, #ffe75c, #73ef75, #63c6f2, #b141ff, #ff4d4d)"
                  : swatch,
              }}
              type="button"
            >
              {isSelected ? (
                <span
                  className="absolute inset-0.5 rounded-full border-2"
                  style={{
                    borderColor: contrastColorForBackground(
                      isCustomSwatch ? selectedColor : swatch,
                    ),
                  }}
                />
              ) : null}
            </button>
          );
        })}
      </div>

      <AvatarCustomColorPanel
        colorDraft={colorDraft}
        hue={customHue}
        onCommit={onCommitCustomColor}
        onHueChange={onHueChange}
        onSaturationValueChange={onSaturationValueChange}
        saturation={customSaturation}
        testIdPrefix={testIdPrefix}
        value={customValue}
        visible={isCustomColorPickerVisible}
      />

      {hasAvatar && onClearAvatar ? (
        <RemoveAvatarButton
          actionButtonClassName={actionButtonClassName}
          assetLabel={assetLabel}
          disabled={disabled}
          onClick={onClearAvatar}
        />
      ) : null}
    </div>
  );
}

export function AgentAvatarAnimatedTab({
  actionButtonClassName,
  assetLabel,
  disabled,
  hasAvatar,
  onApply,
  onApplyPendingChange,
  onClearAvatar,
  onPreviewActiveChange,
  previewContainer,
  processRecording,
  testIdPrefix,
}: {
  actionButtonClassName?: string;
  assetLabel: string;
  disabled: boolean;
  hasAvatar: boolean;
  onApply: (avatarUrl: string) => void;
  onApplyPendingChange: (isPending: boolean) => void;
  onClearAvatar?: () => void;
  onPreviewActiveChange?: (active: boolean) => void;
  previewContainer?: HTMLElement | null;
  processRecording?: AnimatedAvatarRecordingProcessor;
  testIdPrefix: string;
}) {
  return (
    <div className="grid gap-3">
      <AnimatedAvatarCapture
        actionButtonClassName={actionButtonClassName}
        compactReview
        dense
        disabled={disabled}
        onApply={onApply}
        onApplyPendingChange={onApplyPendingChange}
        onPreviewActiveChange={onPreviewActiveChange}
        previewContainer={previewContainer}
        processRecording={processRecording}
        testIdPrefix={`${testIdPrefix}-animated`}
      />
      {hasAvatar && onClearAvatar ? (
        <RemoveAvatarButton
          actionButtonClassName={actionButtonClassName}
          assetLabel={assetLabel}
          disabled={disabled}
          onClick={onClearAvatar}
        />
      ) : null}
    </div>
  );
}
