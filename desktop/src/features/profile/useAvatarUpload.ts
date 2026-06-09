import * as React from "react";

import { uploadMediaBytes } from "@/shared/api/tauri";

const AVATAR_IMAGE_TYPES = [
  "image/gif",
  "image/jpeg",
  "image/png",
  "image/webp",
];

type UseAvatarUploadOptions = {
  onUploadSuccess: (url: string) => void;
};

type UseAvatarUploadReturn = {
  inputRef: React.RefObject<HTMLInputElement | null>;
  isUploading: boolean;
  errorMessage: string | null;
  clearError: () => void;
  openPicker: () => void;
  uploadFile: (file: File) => Promise<void>;
  handleFileChange: (event: React.ChangeEvent<HTMLInputElement>) => void;
};

export function useAvatarUpload({
  onUploadSuccess,
}: UseAvatarUploadOptions): UseAvatarUploadReturn {
  const inputRef = React.useRef<HTMLInputElement | null>(null);
  const [isUploading, setIsUploading] = React.useState(false);
  const [errorMessage, setErrorMessage] = React.useState<string | null>(null);

  const clearError = React.useCallback(() => {
    setErrorMessage(null);
  }, []);

  const openPicker = React.useCallback(() => {
    inputRef.current?.click();
  }, []);

  const uploadFile = React.useCallback(
    async (file: File) => {
      if (!AVATAR_IMAGE_TYPES.includes(file.type)) {
        setErrorMessage("Choose a PNG, JPG, GIF, or WebP image.");
        return;
      }

      setIsUploading(true);
      setErrorMessage(null);

      try {
        const buffer = await file.arrayBuffer();
        const uploaded = await uploadMediaBytes([...new Uint8Array(buffer)]);
        // The shared upload path is now generic (accepts any non-denied file),
        // so the browser-provided `file.type` check above is no longer a
        // backstop. Verify the server-detected MIME is actually an image before
        // accepting it as an avatar — defends against spoofed/blank picker MIME.
        if (!uploaded.type.startsWith("image/")) {
          setErrorMessage("Choose a PNG, JPG, GIF, or WebP image.");
          return;
        }
        onUploadSuccess(uploaded.url);
      } catch (error) {
        setErrorMessage(
          error instanceof Error
            ? error.message
            : "Could not upload that avatar.",
        );
      } finally {
        setIsUploading(false);
      }
    },
    [onUploadSuccess],
  );

  const handleFileChange = React.useCallback(
    (event: React.ChangeEvent<HTMLInputElement>) => {
      const file = event.target.files?.[0];
      event.target.value = "";

      if (!file) {
        return;
      }

      void uploadFile(file);
    },
    [uploadFile],
  );

  return {
    inputRef,
    isUploading,
    errorMessage,
    clearError,
    openPicker,
    uploadFile,
    handleFileChange,
  };
}
