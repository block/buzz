import { Link2, UploadCloud, X } from "lucide-react";

import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { ChannelAvatar } from "@/features/channels/ui/ChannelAvatar";
import type { ChannelType, ChannelVisibility } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Spinner } from "@/shared/ui/spinner";

import {
  CHANNEL_FORM_FIELD_CONTROL_CLASS,
  CHANNEL_FORM_FIELD_SHELL_CLASS,
} from "./channelFormStyles";

type ChannelIconUrlFieldProps = {
  avatarUrl: string;
  channelType: ChannelType;
  disabled?: boolean;
  label?: string;
  name: string;
  onChange: (avatarUrl: string) => void;
  testIdPrefix: string;
  visibility: ChannelVisibility;
};

export function ChannelIconUrlField({
  avatarUrl,
  channelType,
  disabled = false,
  label = "Icon",
  name,
  onChange,
  testIdPrefix,
  visibility,
}: ChannelIconUrlFieldProps) {
  const {
    clearError,
    errorMessage,
    handleFileChange,
    inputRef,
    isUploading,
    openPicker,
  } = useAvatarUpload({
    fallbackErrorMessage: "Could not upload that channel icon.",
    onUploadSuccess: (uploadedUrl) => onChange(uploadedUrl),
  });
  const isDisabled = disabled || isUploading;

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between gap-3">
        <label
          className="text-sm font-medium text-foreground"
          htmlFor={`${testIdPrefix}-icon-url`}
        >
          {label}
          <span className="ml-1 text-xs font-normal text-muted-foreground/50">
            Optional
          </span>
        </label>
        {avatarUrl.trim() ? (
          <Button
            className="h-7 px-2 text-xs"
            data-testid={`${testIdPrefix}-icon-clear`}
            disabled={isDisabled}
            onClick={() => {
              clearError();
              onChange("");
            }}
            type="button"
            variant="ghost"
          >
            <X className="mr-1 h-3.5 w-3.5" />
            Clear
          </Button>
        ) : null}
      </div>

      <div className="flex items-center gap-3">
        <ChannelAvatar
          avatarUrl={avatarUrl.trim() || null}
          channelType={channelType}
          className="h-11 w-11"
          name={name || "Channel"}
          testId={`${testIdPrefix}-icon-preview`}
          visibility={visibility}
        />
        <div className="min-w-0 flex-1 space-y-2">
          <div
            className={cn(
              "flex min-h-11 items-center gap-2 px-3",
              CHANNEL_FORM_FIELD_SHELL_CLASS,
            )}
          >
            <Link2 className="h-4 w-4 shrink-0 text-muted-foreground" />
            <Input
              autoCapitalize="none"
              autoCorrect="off"
              className={cn(
                "h-8 px-0 py-0 leading-6",
                CHANNEL_FORM_FIELD_CONTROL_CLASS,
              )}
              data-testid={`${testIdPrefix}-icon-url`}
              disabled={isDisabled}
              id={`${testIdPrefix}-icon-url`}
              onChange={(event) => {
                clearError();
                onChange(event.target.value);
              }}
              placeholder="https://…"
              spellCheck={false}
              type="url"
              value={avatarUrl}
            />
          </div>
          <div className="flex items-center gap-2">
            <input
              accept="image/gif,image/jpeg,image/png,image/webp"
              className="hidden"
              data-testid={`${testIdPrefix}-icon-file-input`}
              disabled={isDisabled}
              onChange={handleFileChange}
              ref={inputRef}
              type="file"
            />
            <Button
              className="h-8 px-2.5 text-xs"
              data-testid={`${testIdPrefix}-icon-upload`}
              disabled={isDisabled}
              onClick={openPicker}
              type="button"
              variant="outline"
            >
              {isUploading ? (
                <Spinner className="mr-1.5 h-3.5 w-3.5 border-2" />
              ) : (
                <UploadCloud className="mr-1.5 h-3.5 w-3.5" />
              )}
              {isUploading ? "Uploading…" : "Upload image"}
            </Button>
          </div>
        </div>
      </div>

      {errorMessage ? (
        <p
          className="rounded-xl border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive"
          role="alert"
        >
          {errorMessage}
        </p>
      ) : null}
    </div>
  );
}
