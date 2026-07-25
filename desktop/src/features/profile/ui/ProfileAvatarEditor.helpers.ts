import * as React from "react";

import { beginAvatarPresentation } from "@/features/profile/avatarPresentationStore";

export const DONE_BUTTON_CONTENT_TRANSITION = {
  duration: 0.14,
  ease: [0.23, 1, 0.32, 1],
} as const;

export const DONE_BUTTON_SHELL_TRANSITION = {
  duration: 0.18,
  ease: [0.23, 1, 0.32, 1],
} as const;

export function waitForPendingButtonPaint() {
  return new Promise<void>((resolve) => {
    if (
      typeof window === "undefined" ||
      typeof window.requestAnimationFrame !== "function"
    ) {
      setTimeout(resolve, 0);
      return;
    }

    window.requestAnimationFrame(() => {
      window.requestAnimationFrame(() => setTimeout(resolve, 0));
    });
  });
}

export function useUploadPreviewLifecycle({
  clearFallback,
  onSuccess,
  showFallback,
}: {
  clearFallback: () => void;
  onSuccess: (uploadedUrl: string) => void;
  showFallback: (file: File) => void;
}) {
  const pendingFileRef = React.useRef<File | null>(null);

  return {
    onUploadSettled: () => {
      pendingFileRef.current = null;
      clearFallback();
    },
    onUploadStart: (file: File) => {
      pendingFileRef.current = file;
      showFallback(file);
    },
    onUploadSuccess: (uploadedUrl: string) => {
      const pendingFile = pendingFileRef.current;
      if (pendingFile) beginAvatarPresentation(uploadedUrl, pendingFile);
      onSuccess(uploadedUrl);
    },
  };
}

export function useLocalAvatarPreview() {
  const [previewUrl, setPreviewUrl] = React.useState<string | null>(null);
  const previewUrlRef = React.useRef<string | null>(null);

  const clearPreview = React.useCallback(() => {
    if (previewUrlRef.current) URL.revokeObjectURL(previewUrlRef.current);
    previewUrlRef.current = null;
    setPreviewUrl(null);
  }, []);

  const showFilePreview = React.useCallback((file: File) => {
    if (previewUrlRef.current) URL.revokeObjectURL(previewUrlRef.current);
    const nextUrl = URL.createObjectURL(file);
    previewUrlRef.current = nextUrl;
    setPreviewUrl(nextUrl);
  }, []);

  React.useEffect(() => clearPreview, [clearPreview]);

  return { clearPreview, previewUrl, showFilePreview };
}
