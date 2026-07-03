import * as React from "react";
import { Pencil, Plus } from "lucide-react";
import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Spinner } from "@/shared/ui/spinner";

function isAvatarFileDrag(event: React.DragEvent<HTMLElement>) {
  return Array.from(event.dataTransfer.types).includes("Files");
}

const AVATAR_APPLY_MOTION_TRANSITION = {
  duration: 0.14,
  ease: [0.23, 1, 0.32, 1],
} as const;

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
  const [isAvatarUrlInputFocused, setIsAvatarUrlInputFocused] =
    React.useState(false);
  const avatarDragDepthRef = React.useRef(0);
  const shouldReduceMotion = useReducedMotion();
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

  React.useEffect(() => {
    if (isAvatarMenuOpen) {
      setAvatarUrlDraft("");
      setIsAvatarUrlInputFocused(false);
    }
  }, [isAvatarMenuOpen]);

  function applyAvatarUrl() {
    const nextUrl = avatarUrlDraft.trim();
    if (nextUrl.length === 0) {
      return;
    }
    clearUploadError();
    onSelectAvatar(nextUrl);
    setIsAvatarMenuOpen(false);
  }

  const avatarClipStyle = React.useMemo<React.CSSProperties>(
    () => ({
      clipPath: `url(#${avatarEditClipId})`,
      transform: "translateZ(0)",
    }),
    [avatarEditClipId],
  );
  const hasAvatarUrlDraft = avatarUrlDraft.trim().length > 0;
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

  const avatarMenuContent = (
    <PopoverContent align="center" className="w-72 space-y-1 p-1">
      <button
        className="flex min-h-9 w-full items-center rounded-lg px-2.5 text-left text-sm outline-hidden transition-colors duration-150 ease-out hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
        disabled={disabled || isUploading}
        onClick={() => {
          clearUploadError();
          openUploadPicker();
          setIsAvatarMenuOpen(false);
        }}
        type="button"
      >
        Upload an image
      </button>
      <form
        className="flex min-h-9 items-center gap-2 rounded-lg px-2.5 py-1.5 transition-colors duration-150 ease-out focus-within:bg-muted/50"
        onSubmit={(event) => {
          event.preventDefault();
          event.stopPropagation();
          applyAvatarUrl();
        }}
      >
        <label className="sr-only" htmlFor="agent-avatar-url">
          Use a URL
        </label>
        <Input
          autoCapitalize="none"
          autoComplete="off"
          autoCorrect="off"
          className={cn(
            "h-7 min-w-0 flex-1 border-0 bg-transparent px-0 py-0 text-sm shadow-none outline-none focus-visible:ring-0",
            isAvatarUrlInputFocused
              ? "placeholder:text-muted-foreground/55"
              : "placeholder:text-popover-foreground",
          )}
          disabled={disabled || isUploading}
          id="agent-avatar-url"
          onBlur={() => setIsAvatarUrlInputFocused(false)}
          onChange={(event) => setAvatarUrlDraft(event.target.value)}
          onFocus={() => setIsAvatarUrlInputFocused(true)}
          placeholder={isAvatarUrlInputFocused ? "https://..." : "Use a URL"}
          spellCheck={false}
          value={avatarUrlDraft}
        />
        <AnimatePresence initial={false}>
          {hasAvatarUrlDraft ? (
            <motion.div
              animate={{ opacity: 1, scale: 1, width: "auto" }}
              className="overflow-hidden"
              exit={{ opacity: 0, scale: 0.96, width: 0 }}
              initial={{ opacity: 0, scale: 0.96, width: 0 }}
              key="apply-avatar-url"
              transition={applyButtonTransition}
            >
              <Button
                className="h-7 px-2 text-xs"
                disabled={disabled || isUploading}
                size="xs"
                type="submit"
              >
                Apply
              </Button>
            </motion.div>
          ) : null}
        </AnimatePresence>
      </form>
      {hasAvatar && onClearAvatar ? (
        <button
          className="flex min-h-9 w-full items-center rounded-lg px-2.5 text-left text-sm text-destructive outline-hidden transition-colors duration-150 ease-out hover:bg-destructive/10 focus-visible:bg-destructive/10 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50"
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
