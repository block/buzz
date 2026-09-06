import emojiData from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { flushSync } from "react-dom";

import { AnimatedAvatarCapture } from "@/features/profile/ui/AnimatedAvatarCapture";
import { AvatarCustomColorPanel } from "@/features/profile/ui/AvatarCustomColorPanel";
import { ProfileAvatarImagePanel } from "@/features/profile/ui/ProfileAvatarImagePanel";
import { ProfileAvatarModeTabs } from "@/features/profile/ui/ProfileAvatarModeTabs";
import { useAvatarSelection } from "@/features/profile/avatarPresentationStore";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { useEmojiBurst } from "@/shared/ui/EmojiBurstProvider";
import { Spinner } from "@/shared/ui/spinner";
import {
  DONE_BUTTON_CONTENT_TRANSITION,
  DONE_BUTTON_SHELL_TRANSITION,
  useLocalAvatarPreview,
  useUploadPreviewLifecycle,
  waitForPendingButtonPaint,
} from "./ProfileAvatarEditor.helpers";
import {
  AVATAR_COLORS,
  AVATAR_COLOR_SWATCHES,
  CUSTOM_AVATAR_COLOR_SWATCH,
  DEFAULT_CUSTOM_HUE,
  DEFAULT_CUSTOM_SATURATION,
  DEFAULT_CUSTOM_VALUE,
  DEFAULT_EMOJI_AVATAR_COLOR,
  EMOJI_MART_CATEGORIES,
  type AvatarColorSwatch,
  contrastColorForBackground,
  dataTransferHasImage,
  emojiAvatarDataUrl,
  hexToHsv,
  hsvToHex,
  normalizeHue,
  parseEmojiAvatarDataUrl,
  useEmojiMartStyles,
  useEmojiMartThemeVars,
} from "./ProfileAvatarEditor.utils";
export { parseEmojiAvatarDataUrl } from "./ProfileAvatarEditor.utils";
export type { AvatarMode } from "./ProfileAvatarEditor.types";
import type {
  AvatarMode,
  ProfileAvatarEditorProps,
} from "./ProfileAvatarEditor.types";

const INITIAL_EMOJI_AVATAR_COLORS = AVATAR_COLORS.filter(
  (color) => color !== DEFAULT_EMOJI_AVATAR_COLOR,
);

function randomInitialEmojiAvatarColor() {
  const colors =
    INITIAL_EMOJI_AVATAR_COLORS.length > 0
      ? INITIAL_EMOJI_AVATAR_COLORS
      : AVATAR_COLORS;
  return (
    colors[Math.floor(Math.random() * colors.length)] ??
    DEFAULT_EMOJI_AVATAR_COLOR
  );
}

export function ProfileAvatarEditor({
  avatarUrl,
  donePending = false,
  emojiPickerTheme = "dark",
  emojiPickerThemeVars,
  onCustomColorPickerOpenChange,
  onEmojiAvatarChange,
  onModeChange,
  onLocalPreviewChange,
  onUploadedAvatarChange,
  onUrlChange,
  onAnimatedAvatarApply,
  onDone,
  onUploadingChange,
  processAnimatedAvatar,
  processImage,
  showEmojiColorControlsWhenEmpty = false,
  disabled,
  testIdPrefix = "profile-avatar",
  animatedPreviewContainer = null,
  modeTabsContainer,
  modeTabsOrientation = "horizontal",
  onAnimatedPreviewActiveChange,
  onAnimatedPreviewCaptionChange,
  presentation = "default",
}: ProfileAvatarEditorProps) {
  const { burstEmoji } = useEmojiBurst();
  const shouldReduceMotion = useReducedMotion();
  const initialEmojiAvatar = React.useMemo(
    () => parseEmojiAvatarDataUrl(avatarUrl),
    [avatarUrl],
  );
  const [mode, setMode] = React.useState<AvatarMode>("image");
  const [isDragging, setIsDragging] = React.useState(false);
  const [urlDraft, setUrlDraft] = React.useState("");
  const localPreview = useLocalAvatarPreview();
  React.useEffect(() => {
    onLocalPreviewChange?.(localPreview.previewUrl);
  }, [localPreview.previewUrl, onLocalPreviewChange]);
  const [selectedEmoji, setSelectedEmoji] = React.useState<string | null>(
    () => initialEmojiAvatar?.emoji ?? null,
  );
  const [selectedColor, setSelectedColor] = React.useState(
    () => initialEmojiAvatar?.color ?? DEFAULT_EMOJI_AVATAR_COLOR,
  );
  const [customHue, setCustomHue] = React.useState(DEFAULT_CUSTOM_HUE);
  const [customSaturation, setCustomSaturation] = React.useState(
    DEFAULT_CUSTOM_SATURATION,
  );
  const [customValue, setCustomValue] = React.useState(DEFAULT_CUSTOM_VALUE);
  const [isCustomColorPickerOpen, setIsCustomColorPickerOpen] =
    React.useState(false);
  const [isAnimatedCustomColorPickerOpen, setIsAnimatedCustomColorPickerOpen] =
    React.useState(false);
  const dragDepthRef = React.useRef(0);
  const emojiPickerContainerRef = React.useRef<HTMLDivElement | null>(null);
  const modeContentRef = React.useRef<HTMLDivElement | null>(null);
  const isUrlInputFocusedRef = React.useRef(false);
  const hasUserEditedUrlDraftRef = React.useRef(false);
  const [modeContentHeight, setModeContentHeight] = React.useState<
    number | null
  >(null);
  const documentEmojiMartThemeVars = useEmojiMartThemeVars();
  const emojiMartThemeVars = React.useMemo(
    () =>
      ({
        ...(emojiPickerThemeVars ?? documentEmojiMartThemeVars),
        ...(presentation === "onboarding-modal"
          ? {
              "--buzz-emoji-picker-category-icon-size": "18px",
              "--buzz-emoji-picker-fade-height": "56px",
              "--buzz-emoji-picker-fade-opacity": "1",
              "--buzz-emoji-picker-nav-button-size": "32px",
              "--buzz-emoji-picker-nav-padding-x": "12px",
              "--buzz-emoji-picker-padding": "10px",
              "--buzz-emoji-picker-scroll-padding-top": "18px",
            }
          : presentation === "onboarding-inline"
            ? {
                "--buzz-emoji-picker-category-icon-size": "14px",
                "--buzz-emoji-picker-fade-height": "0px",
                "--buzz-emoji-picker-fade-opacity": "0",
                "--buzz-emoji-picker-nav-button-size": "24px",
                "--buzz-emoji-picker-nav-padding-x": "8px",
                "--buzz-emoji-picker-padding": "6px",
                "--buzz-emoji-picker-scroll-padding-top": "6px",
                "--buzz-emoji-picker-search-control-height": "32px",
              }
            : null),
      }) as React.CSSProperties,
    [documentEmojiMartThemeVars, emojiPickerThemeVars, presentation],
  );
  const customColorDraft = React.useMemo(
    () => hsvToHex(customHue, customSaturation, customValue),
    [customHue, customSaturation, customValue],
  );
  const isOnboardingModal = presentation === "onboarding-modal";
  const isOnboardingInline = presentation === "onboarding-inline";
  const isOnboardingSurface = isOnboardingInline || isOnboardingModal;
  const shouldShowColorControls =
    mode === "emoji" &&
    (selectedEmoji !== null || showEmojiColorControlsWhenEmpty);
  const isCustomColorPickerVisible =
    isCustomColorPickerOpen && shouldShowColorControls;
  const isAnyCustomColorPickerVisible =
    isCustomColorPickerVisible || isAnimatedCustomColorPickerOpen;
  const updateMode = React.useCallback(
    (nextMode: AvatarMode) => {
      if (mode === nextMode) {
        return;
      }

      setMode(nextMode);
      onModeChange?.(nextMode);
    },
    [mode, onModeChange],
  );
  const setAvatar = useAvatarSelection(avatarUrl, onUrlChange);
  const handleUploadSuccess = React.useCallback(
    (uploadedUrl: string) => {
      setUrlDraft("");
      onUploadedAvatarChange?.(uploadedUrl);
      setAvatar(uploadedUrl);
      updateMode("image");
    },
    [onUploadedAvatarChange, setAvatar, updateMode],
  );
  const [isAnimatedApplyPending, setIsAnimatedApplyPending] =
    React.useState(false);
  const uploadPreviewLifecycle = useUploadPreviewLifecycle({
    clearFallback: localPreview.clearPreview,
    onSuccess: handleUploadSuccess,
    showFallback: localPreview.showFilePreview,
  });
  const {
    clearError: clearUploadError,
    errorMessage: uploadErrorMessage,
    handleFileChange,
    inputRef: browseInputRef,
    isUploading,
    openPicker,
    uploadFile,
  } = useAvatarUpload({ ...uploadPreviewLifecycle, processImage });
  const isInputDisabled = disabled || isUploading || isAnimatedApplyPending;
  const handleAnimatedApply = React.useCallback(
    (animatedUrl: string) => {
      clearUploadError();
      setUrlDraft("");
      onUploadedAvatarChange?.(animatedUrl);
      setAvatar(animatedUrl);
      onAnimatedAvatarApply?.(animatedUrl);
    },
    [
      clearUploadError,
      onAnimatedAvatarApply,
      onUploadedAvatarChange,
      setAvatar,
    ],
  );
  // Done on the animated tab uploads the pending recording first, then
  // saves. The save is queued through state so it runs on the next render,
  // after the freshly applied avatar URL has propagated into the host's
  // drafts (calling onDone directly would read stale state).
  const animatedApplyRef = React.useRef<(() => Promise<boolean>) | null>(null);
  const [hasAnimatedApply, setHasAnimatedApply] = React.useState(false);
  const registerAnimatedApply = React.useCallback(
    (apply: (() => Promise<boolean>) | null) => {
      animatedApplyRef.current = apply;
      setHasAnimatedApply(apply !== null);
    },
    [],
  );
  const [isAnimatedDoneQueued, setIsAnimatedDoneQueued] = React.useState(false);
  const isDoneButtonPending =
    donePending ||
    isUploading ||
    isAnimatedApplyPending ||
    isAnimatedDoneQueued;
  const handleDoneClick = React.useCallback(() => {
    const applyAnimated = mode === "animated" ? animatedApplyRef.current : null;
    if (applyAnimated) {
      flushSync(() => {
        setIsAnimatedApplyPending(true);
      });
      void waitForPendingButtonPaint()
        .then(() => applyAnimated())
        .then((applied) => {
          if (applied) {
            setIsAnimatedDoneQueued(true);
            return;
          }
        })
        .catch(() => {})
        .finally(() => {
          setIsAnimatedApplyPending(false);
        });
      return;
    }
    onDone?.();
  }, [mode, onDone]);

  React.useEffect(() => {
    if (!isAnimatedDoneQueued) return;
    setIsAnimatedDoneQueued(false);
    onDone?.();
  }, [isAnimatedDoneQueued, onDone]);

  useEmojiMartStyles(
    emojiPickerContainerRef,
    mode === "emoji",
    isOnboardingInline,
  );

  React.useEffect(() => {
    if (mode !== "emoji") return;

    let animationFrame = 0;
    let observer: MutationObserver | null = null;
    const syncSelectedEmojiButton = () => {
      const shadowRoot =
        emojiPickerContainerRef.current?.querySelector(
          "em-emoji-picker",
        )?.shadowRoot;
      if (!shadowRoot) {
        animationFrame = window.requestAnimationFrame(syncSelectedEmojiButton);
        return;
      }

      shadowRoot
        .querySelectorAll('button[data-buzz-selected="true"]')
        .forEach((button) => {
          button.removeAttribute("data-buzz-selected");
        });
      if (selectedEmoji) {
        shadowRoot.querySelectorAll(".category button").forEach((button) => {
          if (button.getAttribute("aria-label") === selectedEmoji) {
            button.setAttribute("data-buzz-selected", "true");
          }
        });
      }

      observer ??= new MutationObserver(syncSelectedEmojiButton);
      observer.observe(shadowRoot, { childList: true, subtree: true });
    };

    animationFrame = window.requestAnimationFrame(syncSelectedEmojiButton);
    return () => {
      window.cancelAnimationFrame(animationFrame);
      observer?.disconnect();
    };
  }, [mode, selectedEmoji]);

  React.useLayoutEffect(() => {
    const node = modeContentRef.current;
    if (!node) return;

    const updateModeContentHeight = () => {
      setModeContentHeight(node.getBoundingClientRect().height);
    };

    updateModeContentHeight();

    const resizeObserver = new ResizeObserver(updateModeContentHeight);
    resizeObserver.observe(node);

    return () => resizeObserver.disconnect();
  }, []);

  React.useLayoutEffect(() => {
    onUploadingChange?.(isUploading || (!onDone && isAnimatedApplyPending));
  }, [isAnimatedApplyPending, isUploading, onDone, onUploadingChange]);

  React.useEffect(() => {
    const emojiAvatar = parseEmojiAvatarDataUrl(avatarUrl);
    if (emojiAvatar) {
      setSelectedEmoji(emojiAvatar.emoji);
      setSelectedColor(emojiAvatar.color);
      return;
    }

    setSelectedEmoji(null);
    setSelectedColor(DEFAULT_EMOJI_AVATAR_COLOR);
    setIsCustomColorPickerOpen(false);
  }, [avatarUrl]);

  React.useEffect(() => {
    if (!shouldShowColorControls) setIsCustomColorPickerOpen(false);
  }, [shouldShowColorControls]);

  React.useLayoutEffect(() => {
    onCustomColorPickerOpenChange?.(isAnyCustomColorPickerVisible);

    return () => {
      onCustomColorPickerOpenChange?.(false);
    };
  }, [isAnyCustomColorPickerVisible, onCustomColorPickerOpenChange]);

  React.useEffect(() => {
    if (!isCustomColorPickerOpen || !selectedEmoji) {
      return;
    }

    const nextAvatarUrl = emojiAvatarDataUrl(selectedEmoji, customColorDraft);
    if (avatarUrl === nextAvatarUrl) {
      return;
    }

    onUploadedAvatarChange?.(null);
    setAvatar(nextAvatarUrl);
  }, [
    avatarUrl,
    customColorDraft,
    isCustomColorPickerOpen,
    onUploadedAvatarChange,
    selectedEmoji,
    setAvatar,
  ]);

  const handleFiles = React.useCallback(
    (files: FileList | null) => {
      const file = files?.[0];
      if (!file || isInputDisabled) {
        return;
      }

      void uploadFile(file);
      updateMode("image");
    },
    [isInputDisabled, updateMode, uploadFile],
  );

  const applyUrl = React.useCallback(() => {
    const nextUrl = urlDraft.trim();
    if (nextUrl.length === 0 || isInputDisabled) {
      hasUserEditedUrlDraftRef.current = false;
      return;
    }

    clearUploadError();
    onUploadedAvatarChange?.(null);
    setAvatar(nextUrl);
    hasUserEditedUrlDraftRef.current = false;
    updateMode("image");
  }, [
    clearUploadError,
    isInputDisabled,
    onUploadedAvatarChange,
    setAvatar,
    updateMode,
    urlDraft,
  ]);

  const applyEmojiAvatar = React.useCallback(
    (emoji: string, color = selectedColor) => {
      setUrlDraft("");
      hasUserEditedUrlDraftRef.current = false;
      onUploadedAvatarChange?.(null);
      setAvatar(emojiAvatarDataUrl(emoji, color));
      onEmojiAvatarChange?.();
    },
    [onEmojiAvatarChange, onUploadedAvatarChange, selectedColor, setAvatar],
  );

  const openCustomColorPicker = React.useCallback(() => {
    const nextColor = hexToHsv(selectedColor);
    setCustomHue(normalizeHue(nextColor.hue));
    setCustomSaturation(nextColor.saturation);
    setCustomValue(nextColor.value);
    setIsCustomColorPickerOpen(true);
  }, [selectedColor]);

  const commitCustomColor = React.useCallback(() => {
    setSelectedColor(customColorDraft);
    if (selectedEmoji) {
      applyEmojiAvatar(selectedEmoji, customColorDraft);
    }
    setIsCustomColorPickerOpen(false);
  }, [applyEmojiAvatar, customColorDraft, selectedEmoji]);

  const handleColorSelect = React.useCallback(
    (swatch: AvatarColorSwatch) => {
      if (disabled) {
        return;
      }

      if (swatch === CUSTOM_AVATAR_COLOR_SWATCH) {
        if (!selectedEmoji) {
          return;
        }
        openCustomColorPicker();
        return;
      }

      setSelectedColor(swatch);
      if (selectedEmoji) {
        applyEmojiAvatar(selectedEmoji, swatch);
      }
    },
    [applyEmojiAvatar, disabled, openCustomColorPicker, selectedEmoji],
  );

  const resetDragState = React.useCallback(() => {
    dragDepthRef.current = 0;
    setIsDragging(false);
  }, []);

  React.useEffect(() => {
    if (!isDragging) {
      return;
    }

    const handleWindowDragEnd = () => resetDragState();
    const handleWindowDrop = () => resetDragState();
    const handleWindowDragLeave = (event: DragEvent) => {
      if (event.clientX <= 0 || event.clientY <= 0) {
        resetDragState();
        return;
      }

      if (
        event.clientX >= window.innerWidth ||
        event.clientY >= window.innerHeight
      ) {
        resetDragState();
      }
    };

    window.addEventListener("dragend", handleWindowDragEnd);
    window.addEventListener("drop", handleWindowDrop);
    window.addEventListener("dragleave", handleWindowDragLeave);

    return () => {
      window.removeEventListener("dragend", handleWindowDragEnd);
      window.removeEventListener("drop", handleWindowDrop);
      window.removeEventListener("dragleave", handleWindowDragLeave);
    };
  }, [isDragging, resetDragState]);

  const isImageDropActive = mode === "image" && isDragging;
  const shouldShowDoneButton =
    onDone &&
    !isAnyCustomColorPickerVisible &&
    (mode !== "animated" || hasAnimatedApply || isDoneButtonPending);
  const isDoneButtonDisabled =
    disabled ||
    isDoneButtonPending ||
    (isOnboardingModal && mode === "animated" && !hasAnimatedApply);
  const modeTabsContent = (
    <ProfileAvatarModeTabs
      disabled={isInputDisabled}
      mode={mode}
      onModeChange={updateMode}
      orientation={modeTabsOrientation}
      portalContainer={modeTabsContainer}
      presentation={presentation}
    />
  );

  return (
    <fieldset
      className={cn(
        "mx-auto w-full border-0 p-0 text-sm",
        isOnboardingSurface
          ? cn(
              "md:ml-0 md:mr-auto",
              isOnboardingInline ? "max-w-none" : "max-w-[456px]",
            )
          : "max-w-[576px]",
      )}
      data-testid={`${testIdPrefix}-editor`}
      disabled={isInputDisabled}
      onDragEnter={(event) => {
        if (!dataTransferHasImage(event.dataTransfer)) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        if (isInputDisabled) {
          return;
        }
        dragDepthRef.current += 1;
        updateMode("image");
        setIsDragging(true);
      }}
      onDragLeave={(event) => {
        if (!isDragging && !dataTransferHasImage(event.dataTransfer)) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        dragDepthRef.current = Math.max(0, dragDepthRef.current - 1);
        if (dragDepthRef.current === 0) {
          setIsDragging(false);
        }
      }}
      onDragOver={(event) => {
        if (!dataTransferHasImage(event.dataTransfer)) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        if (isInputDisabled) {
          return;
        }
        event.dataTransfer.dropEffect = "copy";
        updateMode("image");
        setIsDragging(true);
      }}
      onDrop={(event) => {
        if (!dataTransferHasImage(event.dataTransfer)) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        resetDragState();
        if (isInputDisabled) {
          return;
        }
        void handleFiles(event.dataTransfer.files);
      }}
    >
      <legend className="sr-only">Avatar image picker</legend>
      <div
        className="relative"
        style={
          isOnboardingModal
            ? { minHeight: isAnyCustomColorPickerVisible ? 704 : 454 }
            : undefined
        }
      >
        <div
          className={cn(
            "relative w-full",
            isOnboardingSurface
              ? "flex min-h-[inherit] flex-col"
              : "grid gap-4",
          )}
        >
          {modeTabsContent}

          <div
            className={cn(
              "transition-[height] duration-[250ms] ease-out",
              isOnboardingSurface
                ? cn(
                    "flex min-h-0 items-center overflow-visible",
                    isOnboardingInline ? "mt-3 h-52" : "flex-1",
                    shouldShowColorControls &&
                      (isOnboardingInline ? "py-2" : "py-6"),
                  )
                : "overflow-hidden",
            )}
            data-testid={`${testIdPrefix}-mode-content-shell`}
            style={
              isOnboardingSurface || modeContentHeight === null
                ? undefined
                : { height: modeContentHeight }
            }
          >
            <div
              className={cn(
                "overflow-visible",
                isOnboardingSurface && "w-full",
                isOnboardingInline && "h-full",
                isOnboardingInline &&
                  mode === "animated" &&
                  "flex items-center [&>*]:w-full",
              )}
              ref={modeContentRef}
            >
              {mode === "image" ? (
                <ProfileAvatarImagePanel
                  disabled={isInputDisabled}
                  isDropActive={isImageDropActive}
                  isOnboardingInline={isOnboardingInline}
                  isOnboardingSurface={isOnboardingSurface}
                  isUploading={isUploading}
                  onBrowse={openPicker}
                  onUrlBlur={() => {
                    isUrlInputFocusedRef.current = false;
                    applyUrl();
                  }}
                  onUrlChange={(value) => {
                    clearUploadError();
                    hasUserEditedUrlDraftRef.current = true;
                    setUrlDraft(value);
                    onUploadedAvatarChange?.(null);
                    setAvatar(value);
                  }}
                  onUrlFocus={() => {
                    isUrlInputFocusedRef.current = true;
                  }}
                  onUrlSubmit={applyUrl}
                  testIdPrefix={testIdPrefix}
                  uploadErrorMessage={uploadErrorMessage}
                  urlDraft={urlDraft}
                />
              ) : mode === "animated" ? (
                <AnimatedAvatarCapture
                  dense={isOnboardingInline}
                  disabled={isInputDisabled}
                  onCustomColorPickerOpenChange={
                    setIsAnimatedCustomColorPickerOpen
                  }
                  onApply={handleAnimatedApply}
                  onApplyPendingChange={setIsAnimatedApplyPending}
                  onPreviewActiveChange={onAnimatedPreviewActiveChange}
                  onPreviewCaptionChange={onAnimatedPreviewCaptionChange}
                  previewContainer={animatedPreviewContainer}
                  processRecording={processAnimatedAvatar}
                  registerApply={registerAnimatedApply}
                  compactReview={isOnboardingSurface}
                  showApplyButton={!onDone}
                  testIdPrefix={testIdPrefix}
                />
              ) : (
                <div
                  className={cn(
                    "relative grid content-start",
                    isOnboardingInline &&
                      cn(
                        "h-full gap-2",
                        shouldShowColorControls
                          ? "grid-rows-[6.25rem_auto]"
                          : "grid-rows-[minmax(0,1fr)_auto]",
                      ),
                    !isOnboardingInline && "gap-3",
                  )}
                >
                  <div
                    className={cn(
                      "buzz-emoji-mart relative z-0 overflow-hidden rounded-xl bg-muted transition-colors duration-[250ms] ease-out",
                      isOnboardingInline
                        ? "h-full min-h-0"
                        : isOnboardingModal
                          ? "h-[316px]"
                          : "h-[384px]",
                    )}
                    data-testid={`${testIdPrefix}-emoji-picker`}
                    ref={emojiPickerContainerRef}
                    style={emojiMartThemeVars}
                  >
                    <Picker
                      categories={EMOJI_MART_CATEGORIES}
                      data={emojiData}
                      dynamicWidth
                      emojiButtonRadius="999px"
                      emojiButtonSize={
                        isOnboardingInline ? 48 : isOnboardingModal ? 44 : 64
                      }
                      emojiSize={
                        isOnboardingInline ? 32 : isOnboardingModal ? 28 : 48
                      }
                      icons="outline"
                      navPosition={isOnboardingInline ? "none" : "bottom"}
                      onEmojiSelect={(
                        emoji: { native?: string },
                        event?: MouseEvent,
                      ) => {
                        if (isInputDisabled) {
                          return;
                        }
                        if (!emoji.native) {
                          return;
                        }
                        const nextColor =
                          selectedEmoji === null
                            ? randomInitialEmojiAvatarColor()
                            : selectedColor;
                        if (!isOnboardingSurface) {
                          burstEmoji(emoji.native, event);
                        }
                        setSelectedEmoji(emoji.native);
                        setSelectedColor(nextColor);
                        applyEmojiAvatar(emoji.native, nextColor);
                      }}
                      previewPosition="none"
                      searchPosition="sticky"
                      set="native"
                      skinTonePosition="search"
                      theme={emojiPickerTheme}
                    />
                  </div>

                  <div
                    aria-hidden={!shouldShowColorControls}
                    className={cn(
                      showEmojiColorControlsWhenEmpty
                        ? "overflow-hidden"
                        : "origin-top overflow-hidden transition-[max-height,margin,opacity,transform] duration-[250ms] ease-out",
                      shouldShowColorControls
                        ? cn(
                            "max-h-64 scale-100 opacity-100",
                            isOnboardingInline ? "mt-0" : "mt-3",
                          )
                        : "mt-0 max-h-0 scale-[0.96] opacity-0",
                    )}
                    data-testid={`${testIdPrefix}-color-grid-shell`}
                    inert={shouldShowColorControls ? undefined : true}
                  >
                    <div
                      className={cn(
                        "grid grid-cols-8 justify-items-center rounded-xl bg-muted transition-colors duration-[250ms] ease-out",
                        isOnboardingInline
                          ? "grid-cols-12 gap-1 p-2"
                          : isOnboardingModal
                            ? "gap-2 p-3"
                            : "gap-3 p-4",
                      )}
                      data-testid={`${testIdPrefix}-color-grid`}
                    >
                      {AVATAR_COLOR_SWATCHES.map((swatch) => {
                        const isCustomSwatch =
                          swatch === CUSTOM_AVATAR_COLOR_SWATCH;
                        const isSelected = isCustomSwatch
                          ? !AVATAR_COLORS.some(
                              (color) =>
                                color.toUpperCase() ===
                                selectedColor.toUpperCase(),
                            )
                          : swatch.toUpperCase() ===
                            selectedColor.toUpperCase();

                        return (
                          <button
                            aria-label={
                              isCustomSwatch
                                ? selectedEmoji
                                  ? "Choose custom avatar color"
                                  : "Choose an emoji before custom avatar color"
                                : `Use ${swatch} background`
                            }
                            aria-pressed={isSelected}
                            className={cn(
                              "relative scroll-mb-52 rounded-full border border-border transition-transform duration-200 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                              isOnboardingInline
                                ? "h-5 w-5"
                                : isOnboardingModal
                                  ? "h-7 w-7"
                                  : "h-10 w-10",
                              isCustomSwatch &&
                                !selectedEmoji &&
                                "cursor-not-allowed opacity-45 hover:scale-100 focus-visible:scale-100",
                            )}
                            data-testid={
                              isCustomSwatch
                                ? `${testIdPrefix}-custom-color`
                                : undefined
                            }
                            disabled={isCustomSwatch && !selectedEmoji}
                            key={swatch}
                            onClick={() => handleColorSelect(swatch)}
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
                                className={cn(
                                  "absolute rounded-full border-[3px]",
                                  isOnboardingSurface ? "inset-0.5" : "inset-1",
                                )}
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
                  </div>

                  <AvatarCustomColorPanel
                    colorDraft={customColorDraft}
                    hue={customHue}
                    onCommit={commitCustomColor}
                    onHueChange={setCustomHue}
                    onSaturationValueChange={(nextSaturation, nextValue) => {
                      setCustomSaturation(nextSaturation);
                      setCustomValue(nextValue);
                    }}
                    saturation={customSaturation}
                    testIdPrefix={testIdPrefix}
                    value={customValue}
                    visible={isCustomColorPickerVisible}
                  />
                </div>
              )}
            </div>
          </div>

          <AnimatePresence initial={false}>
            {shouldShowDoneButton ? (
              <Button
                asChild
                className={cn(
                  isOnboardingModal
                    ? "mx-auto mt-0 h-[2.375rem] min-w-24 rounded-full bg-[rgb(var(--buzz-onboarding-avatar-action-bg))] px-6 text-sm font-medium text-[rgb(var(--buzz-onboarding-avatar-action-fg))] hover:bg-[color:rgb(var(--buzz-onboarding-avatar-action-bg)_/_0.9)]"
                    : "mt-2 h-12 w-full rounded-xl",
                )}
              >
                <motion.button
                  animate={{ opacity: 1, scale: 1 }}
                  data-testid={`${testIdPrefix}-done`}
                  disabled={isDoneButtonDisabled}
                  exit={
                    shouldReduceMotion
                      ? { opacity: 0 }
                      : { opacity: 0, scale: 0.96 }
                  }
                  initial={
                    shouldReduceMotion
                      ? { opacity: 0 }
                      : { opacity: 0, scale: 0.98 }
                  }
                  key="done"
                  onClick={handleDoneClick}
                  transition={DONE_BUTTON_SHELL_TRANSITION}
                  type="button"
                >
                  <span className="grid place-items-center">
                    <AnimatePresence initial={false}>
                      {isDoneButtonPending && !isOnboardingModal ? (
                        <motion.span
                          animate={{ opacity: 1, y: 0 }}
                          className="col-start-1 row-start-1 inline-flex items-center justify-center gap-2"
                          exit={
                            shouldReduceMotion
                              ? { opacity: 0, y: 0 }
                              : { opacity: 0, y: -3 }
                          }
                          initial={
                            shouldReduceMotion
                              ? { opacity: 0, y: 0 }
                              : { opacity: 0, y: 3 }
                          }
                          key="pending"
                          transition={DONE_BUTTON_CONTENT_TRANSITION}
                        >
                          <Spinner
                            aria-label="Saving avatar"
                            className="h-4 w-4 border-2"
                          />
                          <span>Saving</span>
                        </motion.span>
                      ) : (
                        <motion.span
                          animate={{ opacity: 1, y: 0 }}
                          className="col-start-1 row-start-1"
                          exit={
                            shouldReduceMotion
                              ? { opacity: 0, y: 0 }
                              : { opacity: 0, y: -3 }
                          }
                          initial={
                            shouldReduceMotion
                              ? { opacity: 0, y: 0 }
                              : { opacity: 0, y: 3 }
                          }
                          key="ready"
                          transition={DONE_BUTTON_CONTENT_TRANSITION}
                        >
                          {isOnboardingModal ? "Save" : "Done"}
                        </motion.span>
                      )}
                    </AnimatePresence>
                  </span>
                </motion.button>
              </Button>
            ) : null}
          </AnimatePresence>
        </div>
      </div>

      <input
        accept="image/*"
        className="hidden"
        data-testid={`${testIdPrefix}-input`}
        onChange={handleFileChange}
        ref={browseInputRef}
        type="file"
      />
    </fieldset>
  );
}
