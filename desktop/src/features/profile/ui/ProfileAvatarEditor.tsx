import emojiData from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import { Link2, UploadCloud } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";
import * as React from "react";
import { createPortal, flushSync } from "react-dom";

import { AnimatedAvatarCapture } from "@/features/profile/ui/AnimatedAvatarCapture";
import { AvatarCustomColorPanel } from "@/features/profile/ui/AvatarCustomColorPanel";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { useEmojiBurst } from "@/shared/ui/EmojiBurstProvider";
import { Spinner } from "@/shared/ui/spinner";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
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

export type AvatarMode = "image" | "emoji" | "animated";

const MODE_TAB_ORDER: AvatarMode[] = ["image", "emoji", "animated"];
const DONE_BUTTON_CONTENT_TRANSITION = {
  duration: 0.14,
  ease: [0.23, 1, 0.32, 1],
} as const;
const DONE_BUTTON_SHELL_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

function waitForPendingButtonPaint() {
  return new Promise<void>((resolve) => {
    if (
      typeof window === "undefined" ||
      typeof window.requestAnimationFrame !== "function"
    ) {
      setTimeout(resolve, 0);
      return;
    }

    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => {
        setTimeout(resolve, 0);
      });
    });
  });
}

type ProfileAvatarEditorProps = {
  avatarUrl: string;
  previewName: string;
  onUrlChange: (url: string) => void;
  emojiPickerTheme?: "auto" | "dark" | "light";
  emojiPickerThemeVars?: React.CSSProperties;
  onEmojiAvatarChange?: () => void;
  onCustomColorPickerOpenChange?: (isOpen: boolean) => void;
  onModeChange?: (mode: AvatarMode) => void;
  onUploadedAvatarChange?: (url: string | null) => void;
  onUploadingChange?: (isUploading: boolean) => void;
  onAnimatedAvatarApply?: (url: string) => void;
  onDone?: () => void;
  donePending?: boolean;
  showEmojiColorControlsWhenEmpty?: boolean;
  disabled?: boolean;
  testIdPrefix?: string;
  /** Host element the animated tab renders its live preview into. */
  animatedPreviewContainer?: HTMLElement | null;
  /** Optional host element for the mode tabs; undefined keeps them inline. */
  modeTabsContainer?: HTMLElement | null;
  /** Fires when the animated tab starts/stops occupying the host preview. */
  onAnimatedPreviewActiveChange?: (active: boolean) => void;
  /** Caption shown under the host preview while animated capture is active. */
  onAnimatedPreviewCaptionChange?: (caption: string | null) => void;
};

type EmojiMartEmoji = {
  native?: string;
};

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
  onUploadedAvatarChange,
  onUrlChange,
  onAnimatedAvatarApply,
  onDone,
  onUploadingChange,
  showEmojiColorControlsWhenEmpty = false,
  disabled,
  testIdPrefix = "profile-avatar",
  animatedPreviewContainer = null,
  modeTabsContainer,
  onAnimatedPreviewActiveChange,
  onAnimatedPreviewCaptionChange,
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
  const emojiMartThemeVars = emojiPickerThemeVars ?? documentEmojiMartThemeVars;
  const customColorDraft = React.useMemo(
    () => hsvToHex(customHue, customSaturation, customValue),
    [customHue, customSaturation, customValue],
  );
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
  const handleUploadSuccess = React.useCallback(
    (uploadedUrl: string) => {
      setUrlDraft("");
      onUploadedAvatarChange?.(uploadedUrl);
      onUrlChange(uploadedUrl);
      updateMode("image");
    },
    [onUploadedAvatarChange, onUrlChange, updateMode],
  );
  const [isAnimatedApplyPending, setIsAnimatedApplyPending] =
    React.useState(false);
  const {
    clearError: clearUploadError,
    errorMessage: uploadErrorMessage,
    handleFileChange,
    inputRef: browseInputRef,
    isUploading,
    openPicker,
    uploadFile,
  } = useAvatarUpload({ onUploadSuccess: handleUploadSuccess });
  const isInputDisabled = disabled || isUploading || isAnimatedApplyPending;
  const handleAnimatedApply = React.useCallback(
    (animatedUrl: string) => {
      clearUploadError();
      setUrlDraft("");
      onUploadedAvatarChange?.(animatedUrl);
      onUrlChange(animatedUrl);
      onAnimatedAvatarApply?.(animatedUrl);
    },
    [
      clearUploadError,
      onAnimatedAvatarApply,
      onUploadedAvatarChange,
      onUrlChange,
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
    if (!isAnimatedDoneQueued) {
      return;
    }
    setIsAnimatedDoneQueued(false);
    onDone?.();
  }, [isAnimatedDoneQueued, onDone]);

  useEmojiMartStyles(emojiPickerContainerRef, mode === "emoji");

  React.useLayoutEffect(() => {
    const node = modeContentRef.current;
    if (!node) {
      return;
    }

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
    if (!shouldShowColorControls) {
      setIsCustomColorPickerOpen(false);
    }
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
    onUrlChange(nextAvatarUrl);
  }, [
    avatarUrl,
    customColorDraft,
    isCustomColorPickerOpen,
    onUploadedAvatarChange,
    onUrlChange,
    selectedEmoji,
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
    onUrlChange(nextUrl);
    hasUserEditedUrlDraftRef.current = false;
    updateMode("image");
  }, [
    clearUploadError,
    isInputDisabled,
    onUploadedAvatarChange,
    onUrlChange,
    updateMode,
    urlDraft,
  ]);

  const applyEmojiAvatar = React.useCallback(
    (emoji: string, color = selectedColor) => {
      setUrlDraft("");
      hasUserEditedUrlDraftRef.current = false;
      onUploadedAvatarChange?.(null);
      onUrlChange(emojiAvatarDataUrl(emoji, color));
      onEmojiAvatarChange?.();
    },
    [onEmojiAvatarChange, onUploadedAvatarChange, onUrlChange, selectedColor],
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
  const modeTabs = (
    <Tabs
      className="w-full"
      onValueChange={(nextMode) => {
        if (isInputDisabled) {
          return;
        }
        updateMode(nextMode as AvatarMode);
      }}
      value={mode}
    >
      <TabsList
        aria-label="Avatar type"
        className="relative isolate grid h-14 w-full grid-cols-3 overflow-hidden rounded-full bg-muted p-1 text-muted-foreground"
      >
        <div
          aria-hidden="true"
          className="absolute bottom-1 left-1 top-1 z-0 rounded-full bg-background shadow transition-transform duration-[250ms] ease-out"
          style={{
            transform: `translateX(${MODE_TAB_ORDER.indexOf(mode) * 100}%)`,
            width: "calc((100% - 8px) / 3)",
          }}
        />
        <TabsTrigger
          className="relative z-10 h-full rounded-full bg-transparent text-sm font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
          disabled={isInputDisabled}
          value="image"
        >
          Image
        </TabsTrigger>
        <TabsTrigger
          className="relative z-10 h-full rounded-full bg-transparent text-sm font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
          disabled={isInputDisabled}
          value="emoji"
        >
          Emoji
        </TabsTrigger>
        <TabsTrigger
          className="relative z-10 h-full rounded-full bg-transparent text-sm font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
          disabled={isInputDisabled}
          value="animated"
        >
          Animated
        </TabsTrigger>
      </TabsList>
    </Tabs>
  );
  const modeTabsContent =
    modeTabsContainer === undefined
      ? modeTabs
      : modeTabsContainer
        ? createPortal(modeTabs, modeTabsContainer)
        : null;

  return (
    <fieldset
      className="mx-auto w-full max-w-[576px] border-0 p-0 text-sm"
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
      <div className="relative">
        <div className="relative grid w-full gap-4">
          {modeTabsContent}

          <div
            className="overflow-hidden transition-[height] duration-[250ms] ease-out"
            style={
              modeContentHeight === null
                ? undefined
                : { height: modeContentHeight }
            }
          >
            <div className="overflow-visible" ref={modeContentRef}>
              {mode === "image" ? (
                <div className="grid content-start gap-3">
                  <button
                    className={cn(
                      "relative flex h-[120px] flex-col items-center justify-center gap-3 overflow-hidden rounded-xl border border-transparent bg-muted text-foreground transition-[background-color,border-color,box-shadow,color] duration-[250ms] ease-out hover:bg-muted/80 disabled:opacity-60",
                      isImageDropActive &&
                        "border-primary bg-primary/10 text-primary ring-1 ring-primary/35 hover:bg-primary/10",
                    )}
                    data-dragging={isImageDropActive ? "true" : undefined}
                    data-testid={`${testIdPrefix}-upload`}
                    disabled={isInputDisabled}
                    onClick={openPicker}
                    type="button"
                  >
                    <span
                      aria-hidden="true"
                      className={cn(
                        "pointer-events-none absolute inset-0 rounded-[inherit] bg-primary/10 opacity-0 transition-opacity duration-[250ms] ease-out",
                        isImageDropActive && "opacity-100",
                      )}
                      data-testid={`${testIdPrefix}-drop-mask`}
                    />
                    {isUploading ? (
                      <Spinner
                        aria-hidden
                        className="relative h-8 w-8 border-2 text-muted-foreground"
                      />
                    ) : (
                      <UploadCloud
                        className={cn(
                          "relative h-8 w-8 text-muted-foreground transition-colors duration-[250ms] ease-out",
                          isImageDropActive && "text-primary",
                        )}
                      />
                    )}
                    <span
                      className={cn(
                        "relative text-sm font-medium text-muted-foreground transition-colors duration-[250ms] ease-out",
                        isImageDropActive && "text-primary",
                      )}
                    >
                      {isUploading ? (
                        "Uploading..."
                      ) : isImageDropActive ? (
                        "Drop image here"
                      ) : (
                        <>
                          Drop or{" "}
                          <span className="underline underline-offset-2">
                            browse
                          </span>
                        </>
                      )}
                    </span>
                  </button>

                  <div className="flex h-16 items-center gap-3 rounded-xl bg-muted px-5 transition-colors duration-[250ms] ease-out focus-within:bg-muted/80">
                    <Link2 className="h-4 w-4 text-muted-foreground" />
                    <input
                      autoCapitalize="none"
                      autoCorrect="off"
                      className="min-w-0 flex-1 bg-transparent text-sm font-medium text-foreground outline-none placeholder:text-muted-foreground"
                      data-testid={`${testIdPrefix}-url`}
                      disabled={isInputDisabled}
                      onBlur={() => {
                        isUrlInputFocusedRef.current = false;
                        applyUrl();
                      }}
                      onChange={(event) => {
                        clearUploadError();
                        hasUserEditedUrlDraftRef.current = true;
                        setUrlDraft(event.target.value);
                        onUploadedAvatarChange?.(null);
                        onUrlChange(event.target.value);
                      }}
                      onFocus={() => {
                        isUrlInputFocusedRef.current = true;
                      }}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          event.preventDefault();
                          applyUrl();
                        }
                      }}
                      placeholder="Paste a URL (Slack profile, etc.)"
                      spellCheck={false}
                      type="url"
                      value={urlDraft}
                    />
                  </div>

                  {uploadErrorMessage ? (
                    <p
                      className="rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm font-medium text-destructive"
                      data-testid={`${testIdPrefix}-upload-error`}
                      role="alert"
                    >
                      {uploadErrorMessage}
                    </p>
                  ) : null}
                </div>
              ) : mode === "animated" ? (
                <AnimatedAvatarCapture
                  disabled={isInputDisabled}
                  onCustomColorPickerOpenChange={
                    setIsAnimatedCustomColorPickerOpen
                  }
                  onApply={handleAnimatedApply}
                  onApplyPendingChange={setIsAnimatedApplyPending}
                  onPreviewActiveChange={onAnimatedPreviewActiveChange}
                  onPreviewCaptionChange={onAnimatedPreviewCaptionChange}
                  previewContainer={animatedPreviewContainer}
                  registerApply={registerAnimatedApply}
                  showApplyButton={!onDone}
                  testIdPrefix={testIdPrefix}
                />
              ) : (
                <div className="relative grid content-start gap-3">
                  <div
                    className="buzz-emoji-mart relative z-0 h-[316px] overflow-hidden rounded-xl bg-muted transition-colors duration-[250ms] ease-out"
                    ref={emojiPickerContainerRef}
                    style={emojiMartThemeVars}
                  >
                    <Picker
                      categories={EMOJI_MART_CATEGORIES}
                      data={emojiData}
                      dynamicWidth
                      emojiButtonRadius="999px"
                      emojiButtonSize={64}
                      emojiSize={48}
                      icons="outline"
                      navPosition="bottom"
                      onEmojiSelect={(
                        emoji: EmojiMartEmoji,
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
                        burstEmoji(emoji.native, event);
                        setSelectedEmoji(emoji.native);
                        setSelectedColor(nextColor);
                        applyEmojiAvatar(emoji.native, nextColor);
                      }}
                      previewPosition="none"
                      searchPosition="none"
                      set="native"
                      skinTonePosition="none"
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
                        ? "mt-3 max-h-64 scale-100 opacity-100"
                        : "mt-0 max-h-0 scale-[0.96] opacity-0",
                    )}
                    data-testid={`${testIdPrefix}-color-grid-shell`}
                    inert={shouldShowColorControls ? undefined : true}
                  >
                    <div
                      className="grid grid-cols-8 justify-items-center gap-3 rounded-xl bg-muted p-4 transition-colors duration-[250ms] ease-out"
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
                              "relative h-10 w-10 scroll-mb-52 rounded-full border border-border transition-transform duration-200 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
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
                                className="absolute inset-1 rounded-full border-[3px]"
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
              <Button asChild className="mt-2 h-12 w-full rounded-xl">
                <motion.button
                  animate={{ opacity: 1, scale: 1 }}
                  data-testid={`${testIdPrefix}-done`}
                  disabled={disabled || isDoneButtonPending}
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
                      {isDoneButtonPending ? (
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
                          Done
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
