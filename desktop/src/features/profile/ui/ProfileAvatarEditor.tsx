import emojiData from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import { Link2, UploadCloud } from "lucide-react";
import { motion } from "motion/react";
import * as React from "react";

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
  CUSTOM_COLOR_GRID_COLUMNS,
  CUSTOM_COLOR_GRID_HORIZONTAL_INSET,
  CUSTOM_COLOR_GRID_ROWS,
  CUSTOM_COLOR_GRID_VERTICAL_INSET,
  CUSTOM_HUE_SCRUBBER_INSET,
  DEFAULT_CUSTOM_HUE,
  DEFAULT_CUSTOM_SATURATION,
  DEFAULT_CUSTOM_VALUE,
  DEFAULT_EMOJI_AVATAR_COLOR,
  EMOJI_MART_CATEGORIES,
  type AvatarColorSwatch,
  clampPercent,
  contrastColorForBackground,
  dataTransferHasImage,
  emojiAvatarDataUrl,
  gridInsetPosition,
  hexToHsv,
  hsvToHex,
  hueScrubberPosition,
  normalizeHue,
  parseEmojiAvatarDataUrl,
  snapToGrid,
  useEmojiMartStyles,
  useEmojiMartThemeVars,
  visibleUrlDraft,
} from "./ProfileAvatarEditor.utils";

export { parseEmojiAvatarDataUrl } from "./ProfileAvatarEditor.utils";

export type AvatarMode = "image" | "emoji";

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
  onDone?: () => void;
  donePending?: boolean;
  hiddenAvatarUrl?: string | null;
  showEmojiColorControlsWhenEmpty?: boolean;
  disabled?: boolean;
  testIdPrefix?: string;
};

type EmojiMartEmoji = {
  native?: string;
};

const AVATAR_EDITOR_MOTION_TRANSITION = {
  duration: 0.25,
  ease: "easeOut",
} as const;
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
  hiddenAvatarUrl,
  onCustomColorPickerOpenChange,
  onEmojiAvatarChange,
  onModeChange,
  onUploadedAvatarChange,
  onUrlChange,
  onDone,
  onUploadingChange,
  showEmojiColorControlsWhenEmpty = false,
  disabled,
  testIdPrefix = "profile-avatar",
}: ProfileAvatarEditorProps) {
  const { burstEmoji } = useEmojiBurst();
  const initialEmojiAvatar = React.useMemo(
    () => parseEmojiAvatarDataUrl(avatarUrl),
    [avatarUrl],
  );
  const [mode, setMode] = React.useState<AvatarMode>("image");
  const [isDragging, setIsDragging] = React.useState(false);
  const [urlDraft, setUrlDraft] = React.useState(() =>
    visibleUrlDraft(avatarUrl, hiddenAvatarUrl),
  );
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
  const dragDepthRef = React.useRef(0);
  const emojiPickerContainerRef = React.useRef<HTMLDivElement | null>(null);
  const hueDragUserSelectRef = React.useRef<string | null>(null);
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
  const {
    clearError: clearUploadError,
    errorMessage: uploadErrorMessage,
    handleFileChange,
    inputRef: browseInputRef,
    isUploading,
    openPicker,
    uploadFile,
  } = useAvatarUpload({ onUploadSuccess: handleUploadSuccess });
  const isInputDisabled = disabled || isUploading;

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
    onUploadingChange?.(isUploading);
  }, [isUploading, onUploadingChange]);

  React.useLayoutEffect(() => {
    if (isUrlInputFocusedRef.current || hasUserEditedUrlDraftRef.current) {
      return;
    }
    setUrlDraft(visibleUrlDraft(avatarUrl, hiddenAvatarUrl));
  }, [avatarUrl, hiddenAvatarUrl]);

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
    onCustomColorPickerOpenChange?.(isCustomColorPickerVisible);

    return () => {
      onCustomColorPickerOpenChange?.(false);
    };
  }, [isCustomColorPickerVisible, onCustomColorPickerOpenChange]);

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

  const unlockHueDragSelection = React.useCallback(() => {
    if (hueDragUserSelectRef.current === null) {
      return;
    }

    document.body.style.userSelect = hueDragUserSelectRef.current;
    hueDragUserSelectRef.current = null;
  }, []);

  const lockHueDragSelection = React.useCallback(() => {
    if (hueDragUserSelectRef.current !== null) {
      return;
    }

    hueDragUserSelectRef.current = document.body.style.userSelect;
    document.body.style.userSelect = "none";
  }, []);

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

  const updateCustomColorFromPointer = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const rect = event.currentTarget.getBoundingClientRect();
      const width = Math.max(
        rect.width - CUSTOM_COLOR_GRID_HORIZONTAL_INSET * 2,
        1,
      );
      const height = Math.max(
        rect.height - CUSTOM_COLOR_GRID_VERTICAL_INSET * 2,
        1,
      );
      const rawSaturation = clampPercent(
        ((event.clientX - rect.left - CUSTOM_COLOR_GRID_HORIZONTAL_INSET) /
          width) *
          100,
      );
      const rawValue = clampPercent(
        (1 -
          (event.clientY - rect.top - CUSTOM_COLOR_GRID_VERTICAL_INSET) /
            height) *
          100,
      );
      const nextSaturation = Math.round(
        snapToGrid(rawSaturation, CUSTOM_COLOR_GRID_COLUMNS),
      );
      const nextValue = Math.round(
        snapToGrid(rawValue, CUSTOM_COLOR_GRID_ROWS),
      );

      setCustomSaturation(nextSaturation);
      setCustomValue(nextValue);
    },
    [],
  );

  const updateCustomHueFromPointer = React.useCallback(
    (event: React.PointerEvent<HTMLDivElement>) => {
      const rect = event.currentTarget.getBoundingClientRect();
      const trackWidth = Math.max(
        rect.width - CUSTOM_HUE_SCRUBBER_INSET * 2,
        1,
      );
      const nextPercent = clampPercent(
        ((event.clientX - rect.left - CUSTOM_HUE_SCRUBBER_INSET) / trackWidth) *
          100,
      );
      setCustomHue(Math.round((nextPercent / 100) * 360));
    },
    [],
  );

  const adjustCustomHue = React.useCallback((delta: number) => {
    setCustomHue((current) => normalizeHue(current + delta));
  }, []);

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
              className="relative isolate grid h-14 w-full grid-cols-2 overflow-hidden rounded-full bg-muted p-1 text-muted-foreground"
            >
              <div
                aria-hidden="true"
                className="absolute bottom-1 left-1 top-1 z-0 rounded-full bg-background shadow transition-transform duration-[250ms] ease-out"
                style={{
                  transform: `translateX(${mode === "emoji" ? "100%" : "0"})`,
                  width: "calc((100% - 8px) / 2)",
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
            </TabsList>
          </Tabs>

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
                        className="relative h-8 w-8 text-muted-foreground"
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

                  <motion.div
                    aria-hidden={!isCustomColorPickerVisible}
                    className={cn(
                      "absolute inset-0 z-40 flex flex-col rounded-xl bg-muted p-4",
                      isCustomColorPickerVisible
                        ? "pointer-events-auto"
                        : "pointer-events-none",
                    )}
                    animate={
                      isCustomColorPickerVisible
                        ? { opacity: 1 }
                        : { opacity: 0 }
                    }
                    inert={isCustomColorPickerVisible ? undefined : true}
                    initial={false}
                    transition={AVATAR_EDITOR_MOTION_TRANSITION}
                  >
                    <div
                      className="relative min-h-0 w-full flex-1 cursor-pointer overflow-hidden rounded-xl shadow-[inset_0_-18px_34px_rgba(0,0,0,0.18)]"
                      data-testid={`${testIdPrefix}-custom-color-spectrum`}
                      onPointerDown={(event) => {
                        event.preventDefault();
                        event.currentTarget.setPointerCapture(event.pointerId);
                        updateCustomColorFromPointer(event);
                      }}
                      onPointerMove={(event) => {
                        if (event.buttons === 1) {
                          event.preventDefault();
                          updateCustomColorFromPointer(event);
                        }
                      }}
                      onPointerUp={(event) => {
                        event.preventDefault();
                        updateCustomColorFromPointer(event);
                      }}
                      style={{
                        backgroundColor: `hsl(${customHue}, 100%, 50%)`,
                        backgroundImage:
                          "linear-gradient(to bottom, transparent 0%, #000000 100%), linear-gradient(to right, #ffffff 0%, rgba(255,255,255,0) 100%)",
                      }}
                    >
                      <div
                        aria-hidden="true"
                        className="pointer-events-none absolute"
                        style={{
                          inset: `${CUSTOM_COLOR_GRID_VERTICAL_INSET}px ${CUSTOM_COLOR_GRID_HORIZONTAL_INSET}px`,
                        }}
                      >
                        {Array.from({
                          length:
                            CUSTOM_COLOR_GRID_COLUMNS * CUSTOM_COLOR_GRID_ROWS,
                        }).map((_, index) => {
                          const column = index % CUSTOM_COLOR_GRID_COLUMNS;
                          const row = Math.floor(
                            index / CUSTOM_COLOR_GRID_COLUMNS,
                          );
                          const gridSaturation = Math.round(
                            (column / (CUSTOM_COLOR_GRID_COLUMNS - 1)) * 100,
                          );
                          const gridValue = Math.round(
                            100 - (row / (CUSTOM_COLOR_GRID_ROWS - 1)) * 100,
                          );
                          const isSelectedGridDot =
                            gridSaturation === customSaturation &&
                            gridValue === customValue;

                          return (
                            <span
                              className={cn(
                                "absolute h-1 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white/60 shadow-[0_0_4px_rgba(255,255,255,0.24)]",
                                isSelectedGridDot &&
                                  "h-3 w-3 border-2 border-white shadow-[0_2px_10px_rgba(0,0,0,0.24)]",
                              )}
                              key={`${column}-${row}`}
                              style={{
                                backgroundColor: isSelectedGridDot
                                  ? customColorDraft
                                  : undefined,
                                left: `${
                                  (column / (CUSTOM_COLOR_GRID_COLUMNS - 1)) *
                                  100
                                }%`,
                                top: `${
                                  (row / (CUSTOM_COLOR_GRID_ROWS - 1)) * 100
                                }%`,
                              }}
                            />
                          );
                        })}
                      </div>
                      <div
                        className="pointer-events-none absolute h-8 w-8 -translate-x-1/2 -translate-y-1/2 rounded-full border-[3px] border-white shadow-[0_5px_16px_rgba(0,0,0,0.24),inset_0_0_0_1px_rgba(0,0,0,0.06)]"
                        style={{
                          backgroundColor: customColorDraft,
                          left: gridInsetPosition(
                            customSaturation,
                            CUSTOM_COLOR_GRID_HORIZONTAL_INSET,
                          ),
                          top: gridInsetPosition(
                            100 - customValue,
                            CUSTOM_COLOR_GRID_VERTICAL_INSET,
                          ),
                        }}
                      />
                    </div>

                    <div
                      aria-label="Choose custom avatar color hue"
                      aria-valuemax={360}
                      aria-valuemin={0}
                      aria-valuenow={customHue}
                      className="buzz-avatar-hue-scrubber relative mt-3 h-10 w-full cursor-pointer select-none rounded-full touch-none"
                      data-testid={`${testIdPrefix}-custom-color-hue`}
                      onKeyDown={(event) => {
                        if (
                          event.key === "ArrowLeft" ||
                          event.key === "ArrowDown"
                        ) {
                          event.preventDefault();
                          adjustCustomHue(-6);
                        } else if (
                          event.key === "ArrowRight" ||
                          event.key === "ArrowUp"
                        ) {
                          event.preventDefault();
                          adjustCustomHue(6);
                        } else if (event.key === "Home") {
                          event.preventDefault();
                          setCustomHue(0);
                        } else if (event.key === "End") {
                          event.preventDefault();
                          setCustomHue(360);
                        }
                      }}
                      onPointerDown={(event) => {
                        event.preventDefault();
                        lockHueDragSelection();
                        event.currentTarget.setPointerCapture(event.pointerId);
                        updateCustomHueFromPointer(event);
                      }}
                      onPointerMove={(event) => {
                        if (event.buttons === 1) {
                          event.preventDefault();
                          updateCustomHueFromPointer(event);
                        }
                      }}
                      onPointerCancel={unlockHueDragSelection}
                      onPointerUp={unlockHueDragSelection}
                      onLostPointerCapture={unlockHueDragSelection}
                      role="slider"
                      tabIndex={isCustomColorPickerVisible ? 0 : -1}
                    >
                      <div
                        aria-hidden="true"
                        className="absolute top-1 h-8 w-8 -translate-x-1/2 rounded-full"
                        data-testid={`${testIdPrefix}-custom-color-hue-thumb`}
                        style={{
                          left: hueScrubberPosition((customHue / 360) * 100),
                        }}
                      >
                        <div className="h-full w-full rounded-full bg-white shadow-[0_5px_18px_rgba(0,0,0,0.24),inset_0_0_0_1px_rgba(0,0,0,0.06)]" />
                      </div>
                    </div>

                    <Button
                      className="mt-3 h-12 w-full rounded-xl"
                      data-testid={`${testIdPrefix}-custom-color-done`}
                      onClick={commitCustomColor}
                      tabIndex={isCustomColorPickerVisible ? 0 : -1}
                      type="button"
                    >
                      Use color
                    </Button>
                  </motion.div>
                </div>
              )}
            </div>
          </div>

          {onDone && !isCustomColorPickerOpen ? (
            <Button
              className="h-12 w-full rounded-xl"
              data-testid={`${testIdPrefix}-done`}
              disabled={disabled || donePending || isUploading}
              onClick={onDone}
              type="button"
            >
              {donePending ? (
                <Spinner
                  aria-label="Saving avatar"
                  className="h-4 w-4 border-2"
                />
              ) : (
                "Done"
              )}
            </Button>
          ) : null}
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
