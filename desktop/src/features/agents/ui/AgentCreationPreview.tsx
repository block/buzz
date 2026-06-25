import * as React from "react";
import { Upload } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";

function isAvatarFileDrag(event: React.DragEvent<HTMLElement>) {
  return Array.from(event.dataTransfer.types).includes("Files");
}

export function AgentCreationPreview({
  avatarUrl,
  disabled = false,
  label,
  onUploadPendingChange,
  onSelectAvatar,
}: {
  avatarUrl: string | null;
  disabled?: boolean;
  label: string;
  onUploadPendingChange?: (isPending: boolean) => void;
  onSelectAvatar: (avatarUrl: string) => void;
}) {
  const [isDragOverAvatar, setIsDragOverAvatar] = React.useState(false);
  const avatarDragDepthRef = React.useRef(0);
  const {
    inputRef: avatarUploadInputRef,
    isUploading,
    errorMessage: uploadErrorMessage,
    clearError: clearUploadError,
    openPicker: openUploadPicker,
    uploadFile: uploadAvatarFile,
    handleFileChange: handleAvatarUploadFileChange,
  } = useAvatarUpload({
    onUploadSuccess: onSelectAvatar,
  });

  React.useEffect(() => {
    onUploadPendingChange?.(isUploading);
    return () => {
      onUploadPendingChange?.(false);
    };
  }, [isUploading, onUploadPendingChange]);

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

  return (
    <div className="mx-auto w-full max-w-[220px] lg:sticky lg:top-0">
      <fieldset
        aria-label="Agent avatar preview"
        className={cn(
          "group/avatar-preview relative m-0 aspect-[4/5] min-h-[240px] min-w-0 overflow-hidden rounded-xl border border-border/70 bg-muted/50 p-0 shadow-xs transition-[background-color,border-color,box-shadow] duration-150",
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

        <div className="absolute inset-0 flex items-center justify-center">
          <ProfileAvatar
            avatarUrl={avatarUrl}
            className="h-36 w-36 text-4xl"
            label={label}
          />
        </div>

        {uploadErrorMessage ? (
          <p className="absolute inset-x-3 bottom-12 rounded-md bg-background/95 px-2 py-1 text-center text-xs text-destructive shadow-xs">
            {uploadErrorMessage}
          </p>
        ) : null}

        <div className="absolute inset-x-3 bottom-3 flex justify-center">
          <button
            className="inline-flex h-8 translate-y-1 items-center justify-center gap-1.5 rounded-full border border-border/70 bg-background/90 px-3 text-xs font-medium text-foreground opacity-0 shadow-xs transition-[background-color,opacity,transform] duration-150 hover:bg-muted focus-visible:translate-y-0 focus-visible:opacity-100 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring group-hover/avatar-preview:translate-y-0 group-hover/avatar-preview:opacity-100 group-focus-within/avatar-preview:translate-y-0 group-focus-within/avatar-preview:opacity-100"
            disabled={disabled || isUploading}
            onClick={() => {
              clearUploadError();
              openUploadPicker();
            }}
            type="button"
          >
            {isUploading ? (
              <Spinner className="h-3.5 w-3.5 border-2" />
            ) : (
              <Upload className="h-3.5 w-3.5" />
            )}
            {isUploading ? "Uploading..." : "Edit avatar"}
          </button>
        </div>
      </fieldset>
    </div>
  );
}
