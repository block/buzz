export type AnimatedAvatarRecordingProcessor = (input: {
  animationBytes: Uint8Array;
  posterBytes: Uint8Array;
}) => Promise<string>;

export type AnimatedAvatarCaptureProps = {
  actionButtonClassName?: string;
  dense?: boolean;
  disabled?: boolean;
  testIdPrefix: string;
  onApply: (avatarUrl: string) => void;
  previewContainer?: HTMLElement | null;
  onPreviewActiveChange?: (active: boolean) => void;
  onPreviewCaptionChange?: (caption: string | null) => void;
  onApplyPendingChange?: (isPending: boolean) => void;
  onCustomColorPickerOpenChange?: (isOpen: boolean) => void;
  registerApply?: (apply: (() => Promise<boolean>) | null) => void;
  showApplyButton?: boolean;
  autoStartCamera?: boolean;
  compactReview?: boolean;
  processRecording?: AnimatedAvatarRecordingProcessor;
};
