import type * as React from "react";

export type AvatarMode = "image" | "emoji" | "animated";
export type AvatarEditorPresentation = "default" | "onboarding-modal";

export type ProfileAvatarEditorProps = {
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
  animatedPreviewContainer?: HTMLElement | null;
  modeTabsContainer?: HTMLElement | null;
  onAnimatedPreviewActiveChange?: (active: boolean) => void;
  onAnimatedPreviewCaptionChange?: (caption: string | null) => void;
  presentation?: AvatarEditorPresentation;
};
