import * as React from "react";
import { Pencil, Plus } from "lucide-react";

import { MaskedAvatarBadgeFrame } from "@/features/profile/ui/MaskedAvatarBadgeFrame";
import type { AnimatedAvatarRecordingProcessor } from "@/features/profile/ui/AnimatedAvatarCapture.types";
import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  AVATAR_COLORS,
  CUSTOM_AVATAR_COLOR_SWATCH,
  DEFAULT_CUSTOM_HUE,
  DEFAULT_CUSTOM_SATURATION,
  DEFAULT_CUSTOM_VALUE,
  DEFAULT_EMOJI_AVATAR_COLOR,
  type AvatarColorSwatch,
  emojiAvatarDataUrl,
  hexToHsv,
  hsvToHex,
  normalizeHue,
  parseEmojiAvatarDataUrl,
  useEmojiMartStyles,
  useEmojiMartThemeVars,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { cn } from "@/shared/lib/cn";
import { useEmojiBurst } from "@/shared/ui/EmojiBurstProvider";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
  PopoverTrigger,
} from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import {
  type AvatarTab,
  type EmojiMartEmoji,
  isAvatarFileDrag,
} from "./AgentCreationPreview.utils";
import {
  AgentAvatarAnimatedTab,
  AgentAvatarEmojiTab,
  AgentAvatarImageTab,
} from "./AgentAvatarPickerTabs";
import { AgentAvatarOnboardingTrigger } from "./AgentAvatarOnboardingTrigger";

const ONBOARDING_EMOJI_MART_THEME_VARS = {
  "--buzz-emoji-picker-rgb-background":
    "var(--buzz-onboarding-emoji-picker-background)",
  "--buzz-emoji-picker-rgb-color": "var(--buzz-onboarding-emoji-picker-color)",
  "--buzz-emoji-picker-rgb-input": "var(--buzz-onboarding-emoji-picker-input)",
} as React.CSSProperties;

export function AgentCreationPreview({
  align = "center",
  allowAnimated = false,
  assetLabel = "avatar",
  avatarUrl,
  disabled = false,
  hideEditControl = false,
  label,
  onClearAvatar,
  onCommitAvatar,
  onUploadPendingChange,
  onSelectAvatar,
  presentation = "default",
  processAnimatedAvatar,
  processImage,
  shape = "circle",
  testIdPrefix = "agent-avatar",
  variant = "default",
}: {
  align?: "center" | "start";
  allowAnimated?: boolean;
  assetLabel?: string;
  avatarUrl: string | null;
  disabled?: boolean;
  /** When true, omit all upload/edit controls and render the avatar as a
   *  plain display element. Use in contexts where avatar editing is
   *  handled by an external affordance (e.g. AgentInstanceEditDialog). */
  hideEditControl?: boolean;
  label: string;
  onClearAvatar?: () => void;
  onCommitAvatar?: (avatarUrl: string) => void;
  onUploadPendingChange?: (isPending: boolean) => void;
  onSelectAvatar: (avatarUrl: string) => void;
  presentation?: "default" | "onboarding";
  processAnimatedAvatar?: AnimatedAvatarRecordingProcessor;
  processImage?: (file: File) => Promise<string>;
  shape?: "circle" | "rounded-square";
  testIdPrefix?: string;
  variant?: "compact" | "default";
}) {
  const [isDragOverAvatar, setIsDragOverAvatar] = React.useState(false);
  const [isAvatarMenuOpen, setIsAvatarMenuOpen] = React.useState(false);
  const [avatarUrlDraft, setAvatarUrlDraft] = React.useState("");
  const [activeTab, setActiveTab] = React.useState<AvatarTab>("image");
  const [selectedEmoji, setSelectedEmoji] = React.useState<string | null>(null);
  const [selectedColor, setSelectedColor] = React.useState(
    DEFAULT_EMOJI_AVATAR_COLOR,
  );
  // Whether the user has explicitly picked a color swatch (vs. the default).
  // The color grid is always visible, so a user can choose a background color
  // before their first emoji — in that case the first emoji must honor the
  // chosen color instead of a random one.
  const [hasChosenColor, setHasChosenColor] = React.useState(false);
  const [customHue, setCustomHue] = React.useState(DEFAULT_CUSTOM_HUE);
  const [customSaturation, setCustomSaturation] = React.useState(
    DEFAULT_CUSTOM_SATURATION,
  );
  const [customValue, setCustomValue] = React.useState(DEFAULT_CUSTOM_VALUE);
  const [isCustomColorPickerOpen, setIsCustomColorPickerOpen] =
    React.useState(false);
  const [isPopoverDragOver, setIsPopoverDragOver] = React.useState(false);
  const [isAnimatedApplyPending, setIsAnimatedApplyPending] =
    React.useState(false);
  const [animatedPreviewContainer, setAnimatedPreviewContainer] =
    React.useState<HTMLDivElement | null>(null);
  const [isAnimatedPreviewActive, setIsAnimatedPreviewActive] =
    React.useState(false);
  const [squishKey, setSquishKey] = React.useState(0);
  const avatarDragDepthRef = React.useRef(0);
  const popoverDragDepthRef = React.useRef(0);
  const emojiPickerContainerRef = React.useRef<HTMLDivElement | null>(null);
  const emojiMartThemeVars = useEmojiMartThemeVars();
  const { burstEmoji } = useEmojiBurst();
  const assetLabelTitle =
    assetLabel.charAt(0).toUpperCase() + assetLabel.slice(1);
  const isRoundedSquare = shape === "rounded-square";
  const isOnboarding = presentation === "onboarding";
  const onboardingAvatarActionButtonClassName = isOnboarding
    ? "h-10 rounded-full px-6 text-sm text-[var(--buzz-onboarding-cta-label)]"
    : undefined;
  const isCompact = variant === "compact";
  const emojiShape = isRoundedSquare ? "rounded-square" : "circle";
  const {
    inputRef: avatarUploadInputRef,
    isUploading,
    errorMessage: uploadErrorMessage,
    clearError: clearUploadError,
    openPicker: openUploadPicker,
    uploadFile: uploadAvatarFile,
    handleFileChange: handleAvatarUploadFileChange,
  } = useAvatarUpload({
    fallbackErrorMessage: `Could not use that ${assetLabel}.`,
    onUploadSuccess: (url) => {
      onSelectAvatar(url);
      onCommitAvatar?.(url);
      setIsAvatarMenuOpen(false);
    },
    processImage,
  });
  useEmojiMartStyles(
    emojiPickerContainerRef,
    isAvatarMenuOpen && activeTab === "emoji",
    false,
    isOnboarding ? "light" : null,
    isOnboarding,
  );
  // Emoji Mart mounts its search input inside a shadow root. Wait for it
  // before focusing so the surrounding Radix popover cannot win the race.
  React.useEffect(() => {
    if (!isAvatarMenuOpen || activeTab !== "emoji") {
      return;
    }

    let animationFrame = 0;
    const focusSearchInput = () => {
      const searchInput =
        emojiPickerContainerRef.current
          ?.querySelector("em-emoji-picker")
          ?.shadowRoot?.querySelector<HTMLInputElement>(
            'input[type="search"]',
          ) ?? null;
      if (!searchInput) {
        animationFrame = window.requestAnimationFrame(focusSearchInput);
        return;
      }
      searchInput.focus();
    };

    animationFrame = window.requestAnimationFrame(focusSearchInput);
    return () => window.cancelAnimationFrame(animationFrame);
  }, [activeTab, isAvatarMenuOpen]);

  const customColorDraft = React.useMemo(
    () => hsvToHex(customHue, customSaturation, customValue),
    [customHue, customSaturation, customValue],
  );

  const isAvatarPending = isUploading || isAnimatedApplyPending;

  React.useEffect(() => {
    onUploadPendingChange?.(isAvatarPending);
    return () => {
      onUploadPendingChange?.(false);
    };
  }, [isAvatarPending, onUploadPendingChange]);

  // Sync emoji state from avatarUrl when the popover opens
  React.useEffect(() => {
    if (isAvatarMenuOpen) {
      setAvatarUrlDraft("");
      setIsPopoverDragOver(false);
      popoverDragDepthRef.current = 0;

      const parsed = parseEmojiAvatarDataUrl(avatarUrl ?? "");
      if (parsed) {
        setSelectedEmoji(parsed.emoji);
        setSelectedColor(parsed.color);
        setHasChosenColor(true);
        setActiveTab("image");
      } else {
        // Non-emoji avatar (image/URL or empty): clear any stale emoji
        // selection so a later color-swatch tap can't re-apply an old emoji
        // over the current avatar.
        setSelectedEmoji(null);
        setSelectedColor(DEFAULT_EMOJI_AVATAR_COLOR);
        setHasChosenColor(false);
        setActiveTab("emoji");
      }
    }
  }, [isAvatarMenuOpen, avatarUrl]);

  // Keep the custom color picker in sync when the selected color changes
  React.useEffect(() => {
    if (!isCustomColorPickerOpen || !selectedEmoji) {
      return;
    }
    const nextAvatarUrl = emojiAvatarDataUrl(
      selectedEmoji,
      customColorDraft,
      emojiShape,
    );
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
    emojiShape,
  ]);

  function applyAvatarUrl() {
    const nextUrl = avatarUrlDraft.trim();
    if (nextUrl.length === 0) {
      return;
    }
    clearUploadError();
    onSelectAvatar(nextUrl);
    onCommitAvatar?.(nextUrl);
    setIsAvatarMenuOpen(false);
  }

  function applyEmojiAvatar(emoji: string, color = selectedColor) {
    const nextAvatarUrl = emojiAvatarDataUrl(emoji, color, emojiShape);
    onSelectAvatar(nextAvatarUrl);
    onCommitAvatar?.(nextAvatarUrl);
    setSquishKey((key) => key + 1);
  }

  function clearAvatar() {
    onClearAvatar?.();
    onCommitAvatar?.("");
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
    setHasChosenColor(true);
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
    setHasChosenColor(true);
    if (selectedEmoji) {
      applyEmojiAvatar(selectedEmoji, customColorDraft);
    }
    setIsCustomColorPickerOpen(false);
  }

  const hasAvatar = (avatarUrl?.trim().length ?? 0) > 0;
  const emojiAvatarPreview = React.useMemo(
    () => parseEmojiAvatarDataUrl(avatarUrl ?? ""),
    [avatarUrl],
  );
  // Outer avatar drag — only active when popover is closed
  const handleAvatarDragEnter = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (disabled || isAvatarMenuOpen || !isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      avatarDragDepthRef.current += 1;
      event.dataTransfer.dropEffect = "copy";
      setIsDragOverAvatar(true);
    },
    [disabled, isAvatarMenuOpen],
  );

  const handleAvatarDragOver = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (disabled || isAvatarMenuOpen || !isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = "copy";
      setIsDragOverAvatar(true);
    },
    [disabled, isAvatarMenuOpen],
  );

  const handleAvatarDragLeave = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (isAvatarMenuOpen || !isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      avatarDragDepthRef.current = Math.max(0, avatarDragDepthRef.current - 1);
      if (avatarDragDepthRef.current === 0) {
        setIsDragOverAvatar(false);
      }
    },
    [isAvatarMenuOpen],
  );

  const handleAvatarDrop = React.useCallback(
    (event: React.DragEvent<HTMLFieldSetElement>) => {
      if (isAvatarMenuOpen || !isAvatarFileDrag(event)) {
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
    [
      clearUploadError,
      disabled,
      isAvatarMenuOpen,
      isUploading,
      uploadAvatarFile,
    ],
  );

  // Popover-level drag — one big drop zone for the entire popover
  const handlePopoverDragEnter = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      if (disabled || !isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      popoverDragDepthRef.current += 1;
      event.dataTransfer.dropEffect = "copy";
      setIsPopoverDragOver(true);
      setActiveTab("image");
    },
    [disabled],
  );

  const handlePopoverDragOver = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      if (disabled || !isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      event.dataTransfer.dropEffect = "copy";
    },
    [disabled],
  );

  const handlePopoverDragLeave = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      if (!isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      popoverDragDepthRef.current = Math.max(
        0,
        popoverDragDepthRef.current - 1,
      );
      if (popoverDragDepthRef.current === 0) {
        setIsPopoverDragOver(false);
      }
    },
    [],
  );

  const handlePopoverDrop = React.useCallback(
    (event: React.DragEvent<HTMLElement>) => {
      if (!isAvatarFileDrag(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      popoverDragDepthRef.current = 0;
      setIsPopoverDragOver(false);

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
      className={cn(
        "w-[400px] p-3",
        isOnboarding &&
          "buzz-onboarding-neutral-theme bg-white text-foreground [--buzz-onboarding-cta-label:#fff]",
      )}
      data-system-color-scheme={isOnboarding ? "light" : undefined}
      side="bottom"
      sideOffset={8}
    >
      {/* Single drop zone covering the entire popover */}
      <fieldset
        aria-label={`${assetLabelTitle} picker`}
        className={cn(
          "relative m-0 rounded-lg border-2 border-transparent p-0 transition-[border-color,background-color] duration-150",
          isPopoverDragOver && "border-dashed border-primary bg-primary/5",
        )}
        onDragEnter={handlePopoverDragEnter}
        onDragLeave={handlePopoverDragLeave}
        onDragOver={handlePopoverDragOver}
        onDrop={handlePopoverDrop}
      >
        <Tabs
          className="w-full"
          onValueChange={(tab) => {
            setActiveTab(tab as AvatarTab);
            setIsCustomColorPickerOpen(false);
          }}
          value={activeTab}
        >
          <TabsList
            className={cn(
              "relative isolate mb-3 grid h-9 w-full overflow-hidden rounded-lg bg-muted p-0.5",
              allowAnimated ? "grid-cols-3" : "grid-cols-2",
            )}
          >
            <div
              aria-hidden="true"
              className="absolute bottom-0.5 left-0.5 top-0.5 z-0 rounded-md bg-background shadow-sm transition-transform duration-200 ease-in-out motion-reduce:transition-none"
              style={{
                transform: `translateX(${activeTab === "image" ? 0 : activeTab === "emoji" ? 100 : 200}%)`,
                width: allowAnimated
                  ? "calc((100% - 4px) / 3)"
                  : "calc((100% - 4px) / 2)",
              }}
            />
            <TabsTrigger
              className="relative z-10 h-full rounded-md bg-transparent text-xs font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:shadow-none"
              value="image"
            >
              Image
            </TabsTrigger>
            <TabsTrigger
              className="relative z-10 h-full rounded-md bg-transparent text-xs font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:shadow-none"
              value="emoji"
            >
              Emoji
            </TabsTrigger>
            {allowAnimated ? (
              <TabsTrigger
                className="relative z-10 h-full rounded-md bg-transparent text-xs font-medium shadow-none transition-colors data-[state=active]:bg-transparent data-[state=active]:shadow-none motion-reduce:transition-none"
                value="animated"
              >
                Animated
              </TabsTrigger>
            ) : null}
          </TabsList>
        </Tabs>

        {activeTab === "image" ? (
          <AgentAvatarImageTab
            actionButtonClassName={onboardingAvatarActionButtonClassName}
            assetLabel={assetLabel}
            avatarUrlDraft={avatarUrlDraft}
            disabled={disabled}
            hasAvatar={hasAvatar}
            isUploading={isUploading}
            onApplyUrl={applyAvatarUrl}
            onAvatarUrlDraftChange={setAvatarUrlDraft}
            onClearAvatar={
              onClearAvatar
                ? () => {
                    clearAvatar();
                    setIsAvatarMenuOpen(false);
                  }
                : undefined
            }
            onOpenUploadPicker={() => {
              clearUploadError();
              openUploadPicker();
            }}
            uploadErrorMessage={uploadErrorMessage}
          />
        ) : null}

        {activeTab === "emoji" ? (
          <AgentAvatarEmojiTab
            actionButtonClassName={onboardingAvatarActionButtonClassName}
            assetLabel={assetLabel}
            colorDraft={customColorDraft}
            customHue={customHue}
            customSaturation={customSaturation}
            customValue={customValue}
            disabled={disabled}
            emojiMartThemeVars={
              isOnboarding
                ? ONBOARDING_EMOJI_MART_THEME_VARS
                : emojiMartThemeVars
            }
            emojiPickerTheme={isOnboarding ? "light" : "auto"}
            emojiPickerContainerRef={emojiPickerContainerRef}
            emojiSearchControlHeight={isOnboarding ? "32px" : undefined}
            hasAvatar={hasAvatar}
            isCustomColorPickerVisible={isCustomColorPickerVisible}
            onClearAvatar={
              onClearAvatar
                ? () => {
                    clearAvatar();
                    setSelectedEmoji(null);
                    setIsAvatarMenuOpen(false);
                  }
                : undefined
            }
            onColorSelect={handleColorSelect}
            onCommitCustomColor={commitCustomColor}
            onEmojiSelect={(emoji: EmojiMartEmoji, event?: MouseEvent) => {
              if (disabled || !emoji.native) {
                return;
              }
              const nextColor =
                selectedEmoji === null && !hasChosenColor
                  ? (AVATAR_COLORS[
                      Math.floor(Math.random() * AVATAR_COLORS.length)
                    ] ?? DEFAULT_EMOJI_AVATAR_COLOR)
                  : selectedColor;
              burstEmoji(emoji.native, event);
              setSelectedEmoji(emoji.native);
              setSelectedColor(nextColor);
              applyEmojiAvatar(emoji.native, nextColor);
            }}
            onHueChange={setCustomHue}
            onSaturationValueChange={(nextSaturation, nextValue) => {
              setCustomSaturation(nextSaturation);
              setCustomValue(nextValue);
            }}
            selectedColor={selectedColor}
            selectedEmoji={selectedEmoji}
            showCategoryNavigation={!isOnboarding}
            showSkinTonePicker={isOnboarding}
            testIdPrefix={testIdPrefix}
          />
        ) : null}

        {activeTab === "animated" && allowAnimated ? (
          <AgentAvatarAnimatedTab
            actionButtonClassName={onboardingAvatarActionButtonClassName}
            assetLabel={assetLabel}
            disabled={disabled}
            hasAvatar={hasAvatar}
            onApply={(nextAvatarUrl) => {
              clearUploadError();
              onSelectAvatar(nextAvatarUrl);
              onCommitAvatar?.(nextAvatarUrl);
              setIsAvatarMenuOpen(false);
            }}
            onApplyPendingChange={setIsAnimatedApplyPending}
            onClearAvatar={
              onClearAvatar
                ? () => {
                    clearAvatar();
                    setIsAvatarMenuOpen(false);
                  }
                : undefined
            }
            onPreviewActiveChange={setIsAnimatedPreviewActive}
            previewContainer={
              isOnboarding ? animatedPreviewContainer : undefined
            }
            processRecording={processAnimatedAvatar}
            testIdPrefix={testIdPrefix}
          />
        ) : null}
      </fieldset>
    </PopoverContent>
  );

  // Display-only path: no upload controls, no pencil badge, no popover.
  // Used when the caller provides its own edit affordance.
  if (hideEditControl) {
    return (
      <div
        className={cn(
          "w-full",
          isCompact
            ? "w-auto"
            : cn(
                "max-w-[220px] lg:sticky lg:top-0",
                align === "center" && "mx-auto",
              ),
        )}
      >
        <div
          className={cn(
            "group/avatar-preview relative m-0 flex min-w-0 flex-col items-center justify-center rounded-xl border border-transparent p-0",
            isCompact ? "min-h-0" : "min-h-[190px] gap-3",
          )}
        >
          <div
            className={cn("relative", isCompact ? "h-16 w-16" : "h-36 w-36")}
          >
            {emojiAvatarPreview ? (
              <div
                aria-label={`${label} ${assetLabel}`}
                className={cn(
                  "relative flex h-full w-full shrink-0 items-center justify-center overflow-hidden shadow-xs transition-[background-color] duration-200 ease-out",
                  isRoundedSquare
                    ? isCompact
                      ? "rounded-2xl"
                      : "rounded-[2rem]"
                    : "rounded-[30%]",
                )}
                role="img"
                style={{ backgroundColor: emojiAvatarPreview.color }}
              >
                <span
                  className={cn(
                    "flex h-full w-full items-center justify-center leading-none",
                    isCompact ? "text-2xl" : "text-[4rem]",
                  )}
                >
                  {emojiAvatarPreview.emoji}
                </span>
              </div>
            ) : isRoundedSquare && avatarUrl ? (
              <img
                alt={`${label} ${assetLabel}`}
                className={cn(
                  "h-full w-full object-cover shadow-xs",
                  isCompact ? "rounded-2xl" : "rounded-[2rem]",
                )}
                src={avatarUrl}
              />
            ) : (
              <ProfileAvatar
                avatarUrl={avatarUrl}
                shape="squircle"
                className={cn(
                  "h-full w-full",
                  isCompact ? "text-base" : "text-4xl",
                )}
                label={label}
              />
            )}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div
      className={cn(
        "w-full",
        isCompact
          ? "w-auto"
          : cn(
              "max-w-[220px] lg:sticky lg:top-0",
              align === "center" && "mx-auto",
            ),
      )}
    >
      <fieldset
        aria-label={`${assetLabelTitle} preview`}
        className={cn(
          "group/avatar-preview relative m-0 flex min-w-0 flex-col justify-center rounded-xl border border-transparent p-0 transition-[background-color,border-color,box-shadow] duration-150",
          align === "start" ? "items-start" : "items-center",
          isCompact ? "min-h-0" : "min-h-[190px] gap-3",
          isDragOverAvatar &&
            !isAvatarMenuOpen &&
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

        <Popover open={isAvatarMenuOpen} onOpenChange={setIsAvatarMenuOpen}>
          <PopoverAnchor asChild>
            <div
              className={cn("relative", isCompact ? "h-16 w-16" : "h-36 w-36")}
            >
              {isOnboarding ? (
                <AgentAvatarOnboardingTrigger
                  assetLabel={assetLabel}
                  avatarUrl={avatarUrl}
                  disabled={disabled}
                  emojiAvatarPreview={emojiAvatarPreview}
                  hasAvatar={hasAvatar}
                  isAnimatedPreviewActive={isAnimatedPreviewActive}
                  isAvatarPending={isAvatarPending}
                  label={label}
                  onPreviewContainerChange={setAnimatedPreviewContainer}
                  squishKey={squishKey}
                  testIdPrefix={testIdPrefix}
                />
              ) : hasAvatar && isRoundedSquare ? (
                <MaskedAvatarBadgeFrame
                  badge={
                    <PopoverTrigger asChild>
                      <button
                        aria-label={`Edit ${assetLabel}`}
                        className={cn(
                          "flex items-center justify-center rounded-full bg-sidebar-active text-sidebar-active-foreground shadow-lg transition-[background-color,scale] duration-150 ease-out hover:scale-[1.04] hover:bg-sidebar-active focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-90 disabled:hover:scale-100",
                          isCompact ? "h-6 w-6" : "h-9 w-9",
                        )}
                        data-testid={`${testIdPrefix}-open`}
                        disabled={disabled || isAvatarPending}
                        title={`Edit ${assetLabel}`}
                        type="button"
                      >
                        {isAvatarPending ? (
                          <Spinner
                            aria-label={`Uploading ${assetLabel}`}
                            className={cn(
                              "border-2",
                              isCompact ? "h-3 w-3" : "h-4 w-4",
                            )}
                          />
                        ) : (
                          <Pencil
                            className={isCompact ? "h-3 w-3" : "h-4 w-4"}
                          />
                        )}
                      </button>
                    </PopoverTrigger>
                  }
                  badgeBox={
                    isCompact
                      ? { bottom: 0, height: 28, right: 0, width: 28 }
                      : { bottom: 0, height: 42, right: 0, width: 42 }
                  }
                  className={isCompact ? "h-16 w-16" : "h-36 w-36"}
                  clipTestId={`${testIdPrefix}-mask`}
                  cornerRadius={isCompact ? 16 : 32}
                  cutout={
                    isCompact
                      ? { cx: 58, cy: 58, r: 16.5 }
                      : { cx: 123, cy: 123, r: 24 }
                  }
                  maskMode={isCompact ? "radial" : "clip-path"}
                  size={isCompact ? 64 : 144}
                >
                  {emojiAvatarPreview ? (
                    <div
                      aria-label={`${label} ${assetLabel}`}
                      className={cn(
                        "relative flex h-full w-full shrink-0 items-center justify-center overflow-hidden shadow-xs transition-[background-color] duration-200 ease-out",
                        isCompact ? "rounded-2xl" : "rounded-[2rem]",
                      )}
                      role="img"
                      style={{
                        backgroundColor: emojiAvatarPreview.color,
                      }}
                    >
                      <span
                        className={cn(
                          "flex h-full w-full items-center justify-center leading-none",
                          isCompact ? "text-2xl" : "text-[4rem]",
                          squishKey > 0 && "buzz-avatar-squish",
                        )}
                        key={squishKey}
                        style={
                          {
                            "--buzz-avatar-emoji-offset-x": "0px",
                            "--buzz-avatar-emoji-offset-y": "0px",
                          } as React.CSSProperties
                        }
                      >
                        {emojiAvatarPreview.emoji}
                      </span>
                    </div>
                  ) : (
                    <img
                      alt={`${label} ${assetLabel}`}
                      className={cn(
                        "h-full w-full object-cover shadow-xs transition-shadow duration-150",
                        isCompact ? "rounded-2xl" : "rounded-[2rem]",
                        isDragOverAvatar &&
                          !isAvatarMenuOpen &&
                          "ring-2 ring-primary/30",
                      )}
                      src={avatarUrl ?? ""}
                    />
                  )}
                </MaskedAvatarBadgeFrame>
              ) : hasAvatar ? (
                <MaskedAvatarBadgeFrame
                  badge={
                    <PopoverTrigger asChild>
                      <button
                        aria-label={`Edit ${assetLabel}`}
                        className={cn(
                          "flex items-center justify-center rounded-full bg-sidebar-active text-sidebar-active-foreground shadow-lg transition-[background-color,scale] duration-150 ease-out hover:scale-[1.04] hover:bg-sidebar-active focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-90 disabled:hover:scale-100",
                          isCompact ? "h-6 w-6" : "h-9 w-9",
                        )}
                        data-testid={`${testIdPrefix}-open`}
                        disabled={disabled || isAvatarPending}
                        title={`Edit ${assetLabel}`}
                        type="button"
                      >
                        {isAvatarPending ? (
                          <Spinner
                            aria-label={`Uploading ${assetLabel}`}
                            className={cn(
                              "border-2",
                              isCompact ? "h-3 w-3" : "h-4 w-4",
                            )}
                          />
                        ) : (
                          <Pencil
                            className={isCompact ? "h-3 w-3" : "h-4 w-4"}
                          />
                        )}
                      </button>
                    </PopoverTrigger>
                  }
                  badgeBox={
                    isCompact
                      ? { bottom: 0, height: 28, right: 0, width: 28 }
                      : { bottom: 0, height: 42, right: 0, width: 42 }
                  }
                  className={isCompact ? "h-16 w-16" : "h-36 w-36"}
                  clipTestId={`${testIdPrefix}-mask`}
                  cornerRadius={(isCompact ? 64 : 144) * 0.3}
                  cutout={
                    isCompact
                      ? { cx: 58, cy: 58, r: 16.5 }
                      : { cx: 123, cy: 123, r: 24 }
                  }
                  maskMode={isCompact ? "radial" : "clip-path"}
                  size={isCompact ? 64 : 144}
                >
                  {emojiAvatarPreview ? (
                    <div
                      aria-label={`${label} ${assetLabel}`}
                      className="relative flex h-full w-full shrink-0 items-center justify-center overflow-hidden rounded-[30%] shadow-xs transition-[background-color] duration-200 ease-out"
                      role="img"
                      style={{
                        backgroundColor: emojiAvatarPreview.color,
                      }}
                    >
                      <span
                        className={cn(
                          "flex h-full w-full items-center justify-center leading-none",
                          isCompact ? "text-2xl" : "text-[4rem]",
                          squishKey > 0 && "buzz-avatar-squish",
                        )}
                        key={squishKey}
                        style={
                          {
                            "--buzz-avatar-emoji-offset-x": "0px",
                            "--buzz-avatar-emoji-offset-y": "0px",
                          } as React.CSSProperties
                        }
                      >
                        {emojiAvatarPreview.emoji}
                      </span>
                    </div>
                  ) : (
                    <ProfileAvatar
                      avatarUrl={avatarUrl}
                      shape="squircle"
                      className={cn(
                        "h-full w-full transition-shadow duration-150",
                        isCompact ? "text-base" : "text-4xl",
                        isDragOverAvatar &&
                          !isAvatarMenuOpen &&
                          "ring-2 ring-primary/30",
                      )}
                      label={label}
                    />
                  )}
                </MaskedAvatarBadgeFrame>
              ) : (
                <PopoverTrigger asChild>
                  <button
                    aria-label={`Add ${assetLabel}`}
                    className={cn(
                      "flex items-center justify-center border-2 border-dashed border-border bg-background text-primary shadow-xs transition-[background-color,border-color,color,box-shadow,scale] duration-150 ease-out hover:scale-[1.02] hover:border-primary/60 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-default disabled:opacity-60 disabled:hover:scale-100",
                      isCompact ? "h-16 w-16" : "h-36 w-36",
                      isRoundedSquare
                        ? isCompact
                          ? "rounded-2xl"
                          : "rounded-[2rem]"
                        : "rounded-[30%]",
                      isDragOverAvatar &&
                        !isAvatarMenuOpen &&
                        "border-primary/70 bg-primary/5 ring-2 ring-primary/15",
                    )}
                    data-testid={`${testIdPrefix}-open`}
                    disabled={disabled || isAvatarPending}
                    title={`Add ${assetLabel}`}
                    type="button"
                  >
                    {isAvatarPending ? (
                      <Spinner
                        aria-label={`Uploading ${assetLabel}`}
                        className="h-4 w-4 border-2"
                      />
                    ) : (
                      <Plus
                        aria-hidden="true"
                        className={isCompact ? "h-6 w-6" : "h-14 w-14"}
                      />
                    )}
                  </button>
                </PopoverTrigger>
              )}
            </div>
          </PopoverAnchor>
          {avatarMenuContent}
        </Popover>

        {uploadErrorMessage ? (
          <p className="max-w-full rounded-md bg-background/95 px-2 py-1 text-center text-xs text-destructive shadow-xs">
            {uploadErrorMessage}
          </p>
        ) : null}
      </fieldset>
    </div>
  );
}
