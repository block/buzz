import * as React from "react";
import { Camera, Link2, Upload, X } from "lucide-react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

type AvatarUploadProps = {
  avatarUrl: string;
  previewName: string;
  onUrlChange: (url: string) => void;
  onClear?: () => void;
  onUploadingChange?: (isUploading: boolean) => void;
  showClear?: boolean;
  disabled?: boolean;
  idleHint?: string;
  testIdPrefix?: string;
};

export function AvatarUpload({
  avatarUrl,
  previewName,
  onUrlChange,
  onClear,
  onUploadingChange,
  showClear,
  disabled,
  idleHint = "",
  testIdPrefix = "avatar",
}: AvatarUploadProps) {
  const [isDragging, setIsDragging] = React.useState(false);

  const onUploadSuccess = React.useCallback(
    (url: string) => {
      onUrlChange(url);
    },
    [onUrlChange],
  );

  const {
    inputRef,
    isUploading,
    errorMessage,
    clearError,
    openPicker,
    handleFileChange,
  } = useAvatarUpload({ onUploadSuccess });

  React.useEffect(() => {
    onUploadingChange?.(isUploading);
  }, [isUploading, onUploadingChange]);

  const isInputDisabled = disabled || isUploading;

  const handleDrop = React.useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsDragging(false);
      const file = e.dataTransfer.files[0];
      if (file && inputRef.current) {
        const dt = new DataTransfer();
        dt.items.add(file);
        inputRef.current.files = dt.files;
        void handleFileChange({
          target: inputRef.current,
        } as React.ChangeEvent<HTMLInputElement>);
      }
    },
    [inputRef, handleFileChange],
  );

  return (
    <div className="space-y-4">
      <p className="text-sm font-medium">Add a profile photo</p>
      <div className="flex items-center gap-4">
        <div className="relative h-20 w-20 shrink-0">
          <ProfileAvatar
            avatarUrl={avatarUrl || null}
            className="h-full w-full text-xl"
            iconClassName="h-6 w-6"
            label={previewName}
            testId={`${testIdPrefix}-preview`}
          />
          {showClear && onClear ? (
            <button
              className="absolute -right-1 -top-1 flex h-6 w-6 items-center justify-center rounded-full border border-background bg-destructive text-destructive-foreground shadow-xs transition-colors hover:bg-destructive/80"
              data-testid={`${testIdPrefix}-clear`}
              onClick={onClear}
              title="Remove photo"
              type="button"
            >
              <X className="h-4 w-4" />
            </button>
          ) : (
            <div className="absolute -bottom-1 -right-1 flex h-8 w-8 items-center justify-center rounded-full border border-background bg-primary text-primary-foreground shadow-xs">
              <Camera className="h-4 w-4" />
            </div>
          )}
        </div>
        <button
          className={`flex flex-1 cursor-pointer flex-col items-center justify-center gap-2 rounded-2xl border-2 border-dashed bg-transparent px-4 py-5 transition-colors ${
            isDragging
              ? "border-primary bg-primary/5"
              : "border-primary/30 hover:border-primary/60 hover:bg-primary/5"
          }`}
          data-testid={`${testIdPrefix}-upload`}
          disabled={isInputDisabled}
          onClick={() => openPicker()}
          onDragEnter={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setIsDragging(true);
          }}
          onDragLeave={(e) => {
            e.preventDefault();
            e.stopPropagation();
            if (e.currentTarget.contains(e.relatedTarget as Node | null))
              return;
            setIsDragging(false);
          }}
          onDragOver={(e) => {
            e.preventDefault();
            e.stopPropagation();
          }}
          onDrop={handleDrop}
          type="button"
        >
          {isUploading ? (
            <Spinner
              aria-hidden
              className="h-4 w-4 border-2 text-muted-foreground"
            />
          ) : (
            <Upload className="h-4 w-4 text-muted-foreground" />
          )}
          <span className="text-xs text-muted-foreground">
            {isUploading ? (
              "Uploading..."
            ) : (
              <>
                Drop an image or{" "}
                <span className="font-medium text-foreground underline underline-offset-2">
                  browse
                </span>
              </>
            )}
          </span>
        </button>
        <input
          accept="image/gif,image/jpeg,image/png,image/webp"
          className="hidden"
          data-testid={`${testIdPrefix}-input`}
          onChange={(event) => {
            void handleFileChange(event);
          }}
          ref={inputRef}
          type="file"
        />
      </div>
      {idleHint ? (
        <p className="text-xs text-muted-foreground">{idleHint}</p>
      ) : null}

      <div className="space-y-1.5">
        <label className="text-sm font-medium" htmlFor={`${testIdPrefix}-url`}>
          Avatar URL
        </label>
        <div className="relative min-w-0">
          <Link2 className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            className="pl-9"
            data-testid={`${testIdPrefix}-url`}
            disabled={isInputDisabled}
            id={`${testIdPrefix}-url`}
            onChange={(event) => {
              clearError();
              onUrlChange(event.target.value);
            }}
            placeholder="https://example.com/avatar.png"
            value={avatarUrl}
          />
        </div>
        <p className="text-xs text-muted-foreground">
          Or paste a direct image URL.
        </p>
      </div>

      {errorMessage ? (
        <p
          className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive"
          data-testid={`${testIdPrefix}-error`}
        >
          {errorMessage}
        </p>
      ) : null}
    </div>
  );
}
