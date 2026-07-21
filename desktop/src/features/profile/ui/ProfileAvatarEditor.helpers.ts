import * as React from "react";

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
  onSettled,
  onStart,
  onSuccess,
  showFallback,
}: {
  clearFallback: () => void;
  onSettled?: (succeeded: boolean) => void;
  onStart?: (file: File) => void;
  onSuccess: (uploadedUrl: string) => void;
  showFallback: (file: File) => void;
}) {
  const succeededRef = React.useRef(false);

  return {
    onUploadSettled: () =>
      onSettled ? onSettled(succeededRef.current) : clearFallback(),
    onUploadStart: (file: File) => {
      succeededRef.current = false;
      (onStart ?? showFallback)(file);
    },
    onUploadSuccess: (uploadedUrl: string) => {
      succeededRef.current = true;
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
