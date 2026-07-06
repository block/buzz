import emojiData from "@emoji-mart/data";
import Picker from "@emoji-mart/react";
import * as React from "react";
import { Link2, Pencil, Plus, UploadCloud } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
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
  emojiAvatarDataUrl,
  hexToHsv,
  hsvToHex,
  normalizeHue,
  parseEmojiAvatarDataUrl,
  useEmojiMartStyles,
  useEmojiMartThemeVars,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { AvatarCustomColorPanel } from "@/features/profile/ui/AvatarCustomColorPanel";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { useEmojiBurst } from "@/shared/ui/EmojiBurstProvider";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";

function isAvatarFileDrag(event: React.DragEvent<HTMLElement>) {
  return Array.from(event.dataTransfer.types).includes("Files");
}

const AVATAR_APPLY_MOTION_TRANSITION = {
  duration: 0.14,
  ease: [0.23, 1, 0.32, 1],
} as const;

type AvatarTab = "image" | "emoji";

type EmojiMartEmoji = {
  native?: string;
};

export function AgentCreationPreview({
  avatarUrl,
  disabled = false,
  label,
  onClearAvatar,
  onUploadPendingChange,
  onSelectAvatar,
}: {
  avatarUrl: string | null;
  disabled?: boolean;
  label: string;
  onClearAvatar?: () => void;
  onUploadPendingChange?: (isPending: boolean) => void;
  onSelectAvatar: (avatarUrl: string) => void;
}) {
  const avatarEditClipId = React.useId().replace(/:/g, "");
  const [isDragOverAvatar, setIsDragOverAvatar] = React.useState(false);
  const [isAvatarMenuOpen, setIsAvatarMenuOpen] = React.useState(false);
  const [avatarUrlDraft, setAvatarUrlDraft] = React.useState("");
  const [activeTab, setActiveTab] = React.useState<AvatarTab>("image");
  const [selectedEmoji, setSelectedEmoji] = React.useState<string | null>(null);
  const [selectedColor, setSelectedColor] = React.useState(
    DEFAULT_EMOJI_AVATAR_COLOR,
  );
  const [customHue, setCustomHue] = React.useState(DEFAULT_CUSTOM_HUE);
  const [customSaturation, setCustomSaturation] = React.useState(
    DEFAULT_CUSTOM_SATURATION,
  );
  const [customValue, setCustomValue] = React.useState(DEFAULT_CUSTOM_VALUE);
  const [isCustomColorPickerOpen, setIsCustomColorPickerOpen] =
    React.useState(false);
  const [isImageDropActive, setIsImageDropActive] = React.useState(false);
  const avatarDragDepthRef = React.useRef(0);
  const imageDragDepthRef = React.useRef(0);
  const shouldReduceMotion = useReducedMotion();
  const emojiPickerContainerRef = React.useRef<HTMLDivElement | null>(null);
  const emojiMartThemeVars = useEmojiMartThemeVars();
  const { burstEmoji } = useEmojiBurst();
  const {
    inputRef: avatarUploadInputRef,
    isUploading,
    errorMessage: uploadErrorMessage,
    clearError: clearUploadError,
    openPicker: openUploadPicker,
    uploadFile: uploadAvatarFile,
    handleFileChange: handleAvatarUploadFileChange,
  } = useAvatarUpload({
    onUploadSuccess: (url) => {
      onSelectAvatar(url);
      setIsAvatarMenuOpen(false);
    },
  });

  useEmojiMartStyles(emojiPickerContainerRef, activeTab === "emoji");

  const customColorDraft = React.useMemo(
    () => hsvToHex(customHue, customSaturation, customValue),
    [customHue, customSaturation, customValue],
  );

  React.useEffect(() => {
    onUploadPendingChange?.(isUploading);
    return () => {
      onUploadPendingChange?.(false);
    };
  }, [isUploading, onUploadPendingChange]);

  // Sync emoji state from avatarUrl when the popover opens
  React.useEffect(() => {
    if (isAvatarMenuOpen) {
      setAvatarUrlDraft("");

      const parsed = parseEmojiAvatarDataUrl(avatarUrl ?? "");
      if (parsed) {
        setSelectedEmoji(parsed.emoji);
        setSelectedColor(parsed.color);
        setActiveTab("emoji");
      } else {
        setActiveTab("image");
      }
    }
  }, [isAvatarMenuOpen, avatarUrl]);

  // Keep the custom color picker in sync when the selected color changes
  React.useEffect(() => {
    if (!isCustomColorPickerOpen || !selectedEmoji) {
      return;
    }
    const nextAvatarUrl = emojiAvatarDataUrl(selectedEmoji, customColorDraft);
    if (avatarUrl === nextAvatarUrl) {
      return;
    }
    onSelectAvatar(nextAvatarUrl);
  }, [
    avatarUrl,
    customColorDraft,
    isCustomColorPickerOpen,
    onSelectAvatar,
    selectedEmoji,
  ]);

  function applyAvatarUrl() {
    const nextUrl = avatarUrlDraft.trim();
    if (nextUrl.length === 0) {
      return;
    }
    clearUploadError();
    onSelectAvatar(nextUrl);
    setIsAvatarMenuOpen(false);
  }

  function applyEmojiAvatar(emoji: string, color = selectedColor) {
    onSelectAvatar(emojiAvatarDataUrl(emoji, color));
  }

  function handleColorSelect(swatch: AvatarColorSwatch) {
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
  }

  function openCustomColorPicker() {
    const nextColor = hexToHsv(selectedColor);
    setCustomHue(normalizeHue(nextColor.hue));
    setCustomSaturation(nextColor.saturation);
    setCustomValue(nextColor.value);
    setIsCustomColorPickerOpen(true);
  }

  function commitCustomColor() {
    setSelectedColor(customColorDraft);
    if (selectedEmoji) {
      applyEmojiAvatar(selectedEmoji, customColorDraft);
    }
    setIsCustomColorPickerOpen(false);
  }

  const avatarClipStyle = React.useMemo<React.CSSProperties>(
    () => ({
      clipPath: `url(#${avatarEditClipId})`,
      transform: "translateZ(0)",
    }),
    [avatarEditClipId],
  );
  const hasAvatar = (avatarUrl?.trim().length ?? 0) > 0;
  const applyButtonTransition = shouldReduceMotion
    ? { duration: 0 }
    : AVATAR_APPLY_MOTION_TRANSITION;

  const handleAvatarDragEnter = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (disabled || !isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      avatarDragDepthRef.current += 1;
      event.dataTransfer.dropEffect = "copy";
      setIsDragOverAvatar(true);
    },
    [disabled],
  );

  const handleAvatarDragOver = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (disabled || !isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = "copy";
      setIsDragOverAvatar(true);
    },
    [disabled],
  );

  const handleAvatarDragLeave = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (!isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      avatarDragDepthRef.current = Math.max(0, avatarDragDepthRef.current - 1);
      if (avatarDragDepthRef.current === 0) {
        setIsDragOverAvatar(false);
      }
    },
    [],
  );

  const handleAvatarDrop = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (!isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      avatarDragDepthRef.current = 0;
      setIsDragOverAvatar(false);

      const file = event.dataTransfer.files[0];
      if (!file || disabled || isUploading) {
        return;
      }

      clearUploadError();
      void uploadAvatarFile(file);
    },
    [clearUploadError, disabled, isUploading, uploadAvatarFile],
  );

  const isCustomColorPickerVisible =
    isCustomColorPickerOpen && selectedEmoji !== null;

  const avatarMenuContent = (
    <PopoverContent
      align="center"
      className="w-[340px] p-3"
      side="bottom"
      sideOffset={8}
    >
      <Tabs
        className="w-full"
        onValueChange={(tab) => {
          setActiveTab(tab as AvatarTab);
          setIsCustomColorPickerOpen(false);
        }}
        value={activeTab}
      >
        <TabsList className="mb-3 grid h-9 w-full grid-cols-2 rounded-lg bg-muted p-0.5">
          <TabsTrigger
            className="h-full rounded-md text-xs font-medium data-[state=active]:bg-background data-[state=active]:shadow-sm"
            value="image"
          >
            Image
          </TabsTrigger>
          <TabsTrigger
            className="h-full rounded-md text-xs font-medium data-[state=active]:bg-background data-[state=active]:shadow-sm"
            value="emoji"
          >
            Emoji
          </TabsTrigger>
        </TabsList>
      </Tabs>

      {activeTab === "image" ? (
        <div className="grid gap-2.5">
          {/* Drop / browse zone — mini version of profile editor */}
          <button
            className={cn(
              "relative flex h-[80px] flex-col items-center justify-center gap-1.5 overflow-hidden rounded-lg border border-transparent bg-muted text-foreground transition-[background-color,border-color,box-shadow,color] duration-200 ease-out hover:bg-muted/80 disabled:opacity-60",
              isImageDropActive &&
                "border-primary bg-primary/10 text-primary ring-1 ring-primary/35 hover:bg-primary/10",
            )}
            disabled={disabled || isUploading}
            onClick={() => {
              clearUploadError();
              openUploadPicker();
            }}
            onDragEnter={(event) => {
              if (disabled || !isAvatarFileDrag(event)) {
                return;
              }
              event.preventDefault();
              event.stopPropagation();
              imageDragDepthRef.current += 1;
              event.dataTransfer.dropEffect = "copy";
              setIsImageDropActive(true);
            }}
            onDragLeave={(event) => {
              if (!isAvatarFileDrag(event)) {
                return;
              }
              event.preventDefault();
              event.stopPropagation();
              imageDragDepthRef.current = Math.max(
                0,
                imageDragDepthRef.current - 1,
              );
              if (imageDragDepthRef.current === 0) {
                setIsImageDropActive(false);
              }
            }}
            onDragOver={(event) => {
              if (disabled || !isAvatarFileDrag(event)) {
                return;
              }
              event.preventDefault();
              event.stopPropagation();
              event.dataTransfer.dropEffect = "copy";
              setIsImageDropActive(true);
            }}
            onDrop={(event) => {
              if (!isAvatarFileDrag(event)) {
                return;
              }
              event.preventDefault();
              event.stopPropagation();
              imageDragDepthRef.current = 0;
              setIsImageDropActive(false);
              const file = event.dataTransfer.files[0];
              if (!file || disabled || isUploading) {
                return;
              }
              clearUploadError();
              void uploadAvatarFile(file);
            }}
            type="button"
          >
            {isUploading ? (
              <Spinner
                aria-hidden
                className="h-5 w-5 border-2 text-muted-foreground"
              />
            ) : (
              <UploadCloud
                className={cn(
                  "h-5 w-5 text-muted-foreground transition-colors duration-200 ease-out",
                  isImageDropActive && "text-primary",
                )}
              />
            )}
            <span
              className={cn(
                "text-xs font-medium text-muted-foreground transition-colors duration-200 ease-out",
                isImageDropActive && "text-primary",
              )}
            >
              {isUploading
                ? "Uploading..."
                : isImageDropActive
                  ? "Drop image here"
                  : "Drop or browse"}
            </span>
          </button>

          {/* URL input — same style as profile editor */}
          <div className="flex h-10 items-center gap-2.5 rounded-lg bg-muted px-3">
            <Link2 className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <input
              autoCapitalize="none"
              autoCorrect="off"
              className="min-w-0 flex-1 bg-transparent text-xs font-medium text-foreground outline-none placeholder:text-muted-foreground"
              disabled={disabled || isUploading}
              onBlur={() => applyAvatarUrl()}
              onChange={(event) => setAvatarUrlDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  applyAvatarUrl();
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
                    onClick={() => applyAvatarUrl()}
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
            <button
              className="flex min-h-8 w-full items-center justify-center rounded-lg text-xs text-destructive outline-hidden transition-colors duration-150 ease-out hover:bg-destructive/10 focus-visible:bg-destructive/10 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
              disabled={disabled || isUploading}
              onClick={() => {
                onClearAvatar();
                setIsAvatarMenuOpen(false);
              }}
              type="button"
            >
              Remove avatar
            </button>
          ) : null}
        </div>
      ) : (
        <div className="grid gap-3">
          {/* Emoji picker */}
          <div
            className="buzz-emoji-mart relative z-0 h-[240px] overflow-hidden rounded-lg bg-muted"
            ref={emojiPickerContainerRef}
            style={emojiMartThemeVars}
          >
            <Picker
              categories={EMOJI_MART_CATEGORIES}
              data={emojiData}
              dynamicWidth
              emojiButtonRadius="999px"
              emojiButtonSize={44}
              emojiSize={32}
              icons="outline"
              navPosition="bottom"
              onEmojiSelect={(emoji: EmojiMartEmoji, event?: MouseEvent) => {
                if (disabled) {
                  return;
                }
                if (!emoji.native) {
                  return;
                }
                const nextColor =
                  selectedEmoji === null
                    ? (AVATAR_COLORS[
                        Math.floor(Math.random() * AVATAR_COLORS.length)
                      ] ?? DEFAULT_EMOJI_AVATAR_COLOR)
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
              theme="auto"
            />
          </div>

          {/* Color swatches — always visible */}
          <div className="grid grid-cols-8 justify-items-center gap-2 rounded-lg bg-muted p-3">
            {AVATAR_COLOR_SWATCHES.map((swatch) => {
              const isCustomSwatch = swatch === CUSTOM_AVATAR_COLOR_SWATCH;
              const isSelected = isCustomSwatch
                ? !AVATAR_COLORS.some(
                    (color) =>
                      color.toUpperCase() === selectedColor.toUpperCase(),
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
                    "relative h-7 w-7 rounded-full border border-border transition-transform duration-150 ease-out hover:scale-[1.15] focus-visible:scale-[1.15] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    isCustomSwatch &&
                      !selectedEmoji &&
                      "cursor-not-allowed opacity-45 hover:scale-100 focus-visible:scale-100",
                  )}
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
            colorDraft={customColorDraft}
            hue={customHue}
            onCommit={commitCustomColor}
            onHueChange={setCustomHue}
            onSaturationValueChange={(nextSaturation, nextValue) => {
              setCustomSaturation(nextSaturation);
              setCustomValue(nextValue);
            }}
            saturation={customSaturation}
            testIdPrefix="agent-avatar"
            value={customValue}
            visible={isCustomColorPickerVisible}
          />

          {hasAvatar && onClearAvatar ? (
            <button
              className="flex min-h-8 w-full items-center justify-center rounded-lg text-xs text-destructive outline-hidden transition-colors duration-150 ease-out hover:bg-destructive/10 focus-visible:bg-destructive/10 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
              disabled={disabled}
              onClick={() => {
                onClearAvatar();
                setSelectedEmoji(null);
                setIsAvatarMenuOpen(false);
              }}
              type="button"
            >
              Remove avatar
            </button>
          ) : null}
        </div>
      )}
    </PopoverContent>
  );

  return (
    <div className="mx-auto w-full max-w-[220px] lg:sticky lg:top-0">
      <fieldset
        aria-label="Agent avatar preview"
        className={cn(
          "group/avatar-preview relative m-0 flex min-h-[190px] min-w-0 flex-col items-center justify-center gap-3 rounded-xl border border-transparent p-0 transition-[background-color,border-color,box-shadow] duration-150",
          isDragOverAvatar &&
            "border-dashed border-primary/70 bg-primary/5 ring-2 ring-primary/15",
        )}
        onDragEnter={handleAvatarDragEnter}
        onDragLeave={handleAvatarDragLeave}
        onDragOver={handleAvatarDragOver}
        onDrop={handleAvatarDrop}
      >
        <input
          accept="image/gif,image/jpeg,image/png,image/webp"
          className="hidden"
          onChange={handleAvatarUploadFileChange}
          ref={avatarUploadInputRef}
          type="file"
        />

        <div className="relative h-36 w-36">
          {hasAvatar ? (
            <>
              <svg
                aria-hidden="true"
                className="pointer-events-none absolute inset-0 h-full w-full"
                fill="none"
                height="144"
                viewBox="0 0 144 144"
                width="144"
                xmlns="http://www.w3.org/2000/svg"
              >
                <clipPath clipPathUnits="userSpaceOnUse" id={avatarEditClipId}>
                  <path
                    clipRule="evenodd"
                    d="M100.734 83.3298C102.415 84.1574 104.616 83.8757 105.495 82.2207C109.647 74.3981 112 65.4738 112 56C112 25.0721 86.9279 0 56 0C25.0721 0 0 25.0721 0 56C0 86.9279 25.0721 112 56 112C65.4738 112 74.3981 109.647 82.2207 105.495C83.8757 104.616 84.1574 102.415 83.3298 100.734C82.4783 99.0047 82 97.0582 82 95C82 87.8203 87.8203 82 95 82C97.0582 82 99.0047 82.4783 100.734 83.3298Z"
                    fillRule="evenodd"
                    transform="translate(-25.875 -25.875) scale(1.575)"
                  />
                </clipPath>
              </svg>

              <div className="relative h-full w-full" style={avatarClipStyle}>
                <ProfileAvatar
                  avatarUrl={avatarUrl}
                  className={cn(
                    "h-full w-full text-4xl transition-shadow duration-150",
                    isDragOverAvatar && "ring-2 ring-primary/30",
                  )}
                  label={label}
                />
              </div>

              <div className="absolute bottom-0 right-0 z-10 flex h-[42px] w-[42px] items-center justify-center rounded-full bg-background">
                <Popover
                  open={isAvatarMenuOpen}
                  onOpenChange={setIsAvatarMenuOpen}
                >
                  <PopoverTrigger asChild>
                    <button
                      aria-label="Edit avatar"
                      className="flex h-9 w-9 items-center justify-center rounded-full bg-sidebar-active text-sidebar-active-foreground shadow-lg transition-[background-color,scale] duration-150 ease-out hover:scale-[1.04] hover:bg-sidebar-active focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-90 disabled:hover:scale-100"
                      disabled={disabled || isUploading}
                      title="Edit avatar"
                      type="button"
                    >
                      {isUploading ? (
                        <Spinner
                          aria-label="Uploading avatar"
                          className="h-4 w-4 border-2"
                        />
                      ) : (
                        <Pencil className="h-4 w-4" />
                      )}
                    </button>
                  </PopoverTrigger>
                  {avatarMenuContent}
                </Popover>
              </div>
            </>
          ) : (
            <Popover open={isAvatarMenuOpen} onOpenChange={setIsAvatarMenuOpen}>
              <PopoverTrigger asChild>
                <button
                  aria-label="Add avatar"
                  className={cn(
                    "flex h-full w-full items-center justify-center rounded-full border-2 border-dashed border-border bg-background text-primary shadow-xs transition-[background-color,border-color,color,box-shadow] duration-150 ease-out hover:border-primary/50 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-70",
                    isDragOverAvatar &&
                      "border-primary/70 bg-primary/5 ring-2 ring-primary/15",
                  )}
                  disabled={disabled || isUploading}
                  title="Add avatar"
                  type="button"
                >
                  {isUploading ? (
                    <Spinner
                      aria-label="Uploading avatar"
                      className="h-4 w-4 border-2"
                    />
                  ) : (
                    <Plus aria-hidden="true" className="h-14 w-14" />
                  )}
                </button>
              </PopoverTrigger>
              {avatarMenuContent}
            </Popover>
          )}
        </div>

        {uploadErrorMessage ? (
          <p className="max-w-full rounded-md bg-background/95 px-2 py-1 text-center text-xs text-destructive shadow-xs">
            {uploadErrorMessage}
          </p>
        ) : null}
      </fieldset>
    </div>
  );
}
