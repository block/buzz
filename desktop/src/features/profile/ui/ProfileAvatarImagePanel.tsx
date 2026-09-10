import { Link2, UploadCloud } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";

type ProfileAvatarImagePanelProps = {
  disabled: boolean;
  isDropActive: boolean;
  isOnboardingInline: boolean;
  isOnboardingSurface: boolean;
  isUploading: boolean;
  onBrowse: () => void;
  onUrlBlur: () => void;
  onUrlChange: (value: string) => void;
  onUrlFocus: () => void;
  onUrlSubmit: () => void;
  testIdPrefix: string;
  uploadErrorMessage: string | null;
  urlDraft: string;
};

export function ProfileAvatarImagePanel({
  disabled,
  isDropActive,
  isOnboardingInline,
  isOnboardingSurface,
  isUploading,
  onBrowse,
  onUrlBlur,
  onUrlChange,
  onUrlFocus,
  onUrlSubmit,
  testIdPrefix,
  uploadErrorMessage,
  urlDraft,
}: ProfileAvatarImagePanelProps) {
  return (
    <div
      className={cn(
        "grid content-start gap-3",
        isOnboardingInline && "h-full grid-rows-[minmax(0,1fr)_2.5rem]",
      )}
    >
      <button
        className={cn(
          isOnboardingSurface
            ? cn(
                "relative flex flex-col items-center justify-center overflow-hidden rounded-lg border border-dashed bg-transparent transition-[background-color,border-color,box-shadow,color] duration-[250ms] ease-out disabled:opacity-60",
                isOnboardingInline
                  ? "h-full border-[#d4d4d4] text-foreground hover:bg-[#f5f5f5]"
                  : "h-32 border-[color:rgb(var(--buzz-onboarding-avatar-control-fg)_/_0.7)] text-[rgb(var(--buzz-onboarding-avatar-control-fg))] hover:bg-[color:rgb(var(--buzz-onboarding-avatar-accent-bg)_/_0.18)]",
              )
            : "relative flex h-[120px] flex-col items-center justify-center gap-3 overflow-hidden rounded-xl border border-transparent bg-muted text-foreground transition-[background-color,border-color,box-shadow,color] duration-[250ms] ease-out hover:bg-muted/80 disabled:opacity-60",
          isDropActive &&
            (isOnboardingSurface
              ? isOnboardingInline
                ? "border-foreground bg-[#f5f5f5]"
                : "border-[rgb(var(--buzz-onboarding-avatar-control-fg))] bg-[color:rgb(var(--buzz-onboarding-avatar-accent-bg)_/_0.24)]"
              : "border-primary bg-primary/10 text-primary ring-1 ring-primary/35 hover:bg-primary/10"),
        )}
        data-dragging={isDropActive ? "true" : undefined}
        data-testid={`${testIdPrefix}-upload`}
        disabled={disabled}
        onClick={onBrowse}
        type="button"
      >
        <span
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute inset-0 rounded-[inherit] bg-primary/10 opacity-0 transition-opacity duration-[250ms] ease-out",
            isDropActive && "opacity-100",
          )}
          data-testid={`${testIdPrefix}-drop-mask`}
        />
        {isOnboardingSurface ? null : isUploading ? (
          <Spinner
            aria-hidden
            className="relative h-8 w-8 border-2 text-muted-foreground"
          />
        ) : (
          <UploadCloud
            className={cn(
              "relative h-8 w-8 text-muted-foreground transition-colors duration-[250ms] ease-out",
              isDropActive && "text-primary",
            )}
          />
        )}
        <span
          className={cn(
            "relative transition-colors duration-[250ms] ease-out",
            isOnboardingSurface
              ? cn(
                  "text-sm font-normal",
                  isOnboardingInline
                    ? "text-foreground"
                    : "text-[rgb(var(--buzz-onboarding-avatar-control-fg))]",
                )
              : "text-sm font-medium text-muted-foreground",
            isDropActive &&
              (isOnboardingSurface
                ? isOnboardingInline
                  ? "text-foreground"
                  : "text-[rgb(var(--buzz-onboarding-avatar-control-fg))]"
                : "text-primary"),
          )}
        >
          {isUploading ? (
            "Uploading..."
          ) : isDropActive ? (
            "Drop image here"
          ) : isOnboardingSurface ? (
            "Drag or browse"
          ) : (
            <>
              Drop or{" "}
              <span className="underline underline-offset-2">browse</span>
            </>
          )}
        </span>
      </button>

      <div
        className={cn(
          "flex items-center transition-colors duration-[250ms] ease-out",
          isOnboardingSurface
            ? cn(
                "rounded-lg border bg-transparent",
                isOnboardingInline
                  ? "h-10 border-[#d4d4d4] px-3 focus-within:border-[#0f0f0f]"
                  : "h-[52px] border-[color:rgb(var(--buzz-onboarding-avatar-control-fg)_/_0.45)] px-5 focus-within:border-[rgb(var(--buzz-onboarding-avatar-control-fg))]",
              )
            : "h-16 gap-3 rounded-xl bg-muted px-5 focus-within:bg-muted/80",
        )}
      >
        {isOnboardingSurface ? null : (
          <Link2 className="h-4 w-4 text-muted-foreground" />
        )}
        <input
          autoCapitalize="none"
          autoCorrect="off"
          className={cn(
            "min-w-0 flex-1 bg-transparent outline-none",
            isOnboardingSurface
              ? cn(
                  "font-normal text-foreground",
                  isOnboardingInline
                    ? "text-left text-sm placeholder:text-muted-foreground"
                    : "text-center text-sm placeholder:text-[color:rgb(var(--buzz-onboarding-avatar-control-fg)_/_0.55)]",
                )
              : "text-sm font-medium text-foreground placeholder:text-muted-foreground",
          )}
          data-testid={`${testIdPrefix}-url`}
          disabled={disabled}
          onBlur={onUrlBlur}
          onChange={(event) => onUrlChange(event.target.value)}
          onFocus={onUrlFocus}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              onUrlSubmit();
            }
          }}
          placeholder={
            isOnboardingSurface
              ? "Paste a URL"
              : "Paste a URL (Slack profile, etc.)"
          }
          spellCheck={false}
          type="url"
          value={urlDraft}
        />
      </div>

      {uploadErrorMessage ? (
        <p
          className="rounded-xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm font-medium text-destructive"
          data-testid={`${testIdPrefix}-upload-error`}
          role="alert"
        >
          {uploadErrorMessage}
        </p>
      ) : null}
    </div>
  );
}
