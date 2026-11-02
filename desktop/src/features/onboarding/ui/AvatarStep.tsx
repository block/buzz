import * as React from "react";

import { ProfileAvatar } from "@/features/profile/ui/ProfileAvatar";
import {
  emojiAvatarDataUrl,
  parseEmojiAvatarDataUrl,
} from "@/features/profile/ui/ProfileAvatarEditor.utils";
import { useAvatarUpload } from "@/features/profile/useAvatarUpload";
import { FirstRunError, FirstRunLink } from "./firstRunControls";
import { CameraIcon, UploadIcon } from "./firstRunIcons";
import { PhotoCaptureDialog } from "./PhotoCaptureDialog";
import { ErrorBanner } from "./ProfileStep";
import type { ProfileStepActions, ProfileStepState } from "./types";

type AvatarStepProps = {
  actions: {
    advanceWithoutSaving: ProfileStepActions["advanceWithoutSaving"];
    back: () => void;
    onUploadingChange: ProfileStepActions["onUploadingChange"];
    skipForNow: ProfileStepActions["skipForNow"];
    submit: ProfileStepActions["submit"];
    updateAvatarUrl: ProfileStepActions["updateAvatarUrl"];
  };
  /** A "Skip for now" that always shows, not only after an error. */
  showAlwaysSkip?: boolean;
  /**
   * Turns a chosen photo into an inline picture instead of uploading it, for
   * the step that comes before any relay exists.
   */
  processImage?: (file: File) => Promise<string>;
  state: Pick<
    ProfileStepState,
    "avatar" | "isSaving" | "isUploadingAvatar" | "name" | "saveRecovery"
  >;
};

/** The six pastel colours the avatar can take, as the board draws them. */
export const AVATAR_SWATCHES = [
  { name: "Warm sand", color: "#EFE7E1" },
  { name: "Lilac", color: "#EFEAFB" },
  { name: "Sky", color: "#E4ECF6" },
  { name: "Sage", color: "#EAF1E6" },
  { name: "Peach", color: "#F6E7DD" },
  { name: "Rose", color: "#FBE6EC" },
] as const;

const DEFAULT_SWATCH = "#EFEAFB";

/** The first letters of the first and last word in a display name. */
export function initialsOf(name: string): string {
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  const first = words[0].charAt(0);
  const last = words.length > 1 ? words[words.length - 1].charAt(0) : "";
  return `${first}${last}`.toUpperCase();
}

/** Step 2 of 2: a colour or a photo for the avatar. */
export function AvatarStep({
  actions,
  showAlwaysSkip = false,
  processImage,
  state,
}: AvatarStepProps) {
  const { isSaving, isUploadingAvatar, name, saveRecovery } = state;
  const draftUrl = state.avatar.draftUrl;
  const displayName = name.draftValue.trim() || name.savedValue.trim();
  const initials = initialsOf(displayName);
  const parsed = parseEmojiAvatarDataUrl(draftUrl);
  const hasImage = draftUrl.trim().length > 0 && !parsed;
  const activeColor = parsed?.color ?? (hasImage ? null : DEFAULT_SWATCH);
  const [cameraOpen, setCameraOpen] = React.useState(false);
  const upload = useAvatarUpload({
    fallbackErrorMessage: "Could not upload that photo.",
    onUploadSuccess: actions.updateAvatarUrl,
    processImage,
  });
  const { isUploading } = upload;
  const { onUploadingChange } = actions;
  React.useEffect(() => {
    onUploadingChange(isUploading);
  }, [isUploading, onUploadingChange]);
  const busy = isSaving || isUploadingAvatar || isUploading;

  // The board starts on a colour, so Next saves that colour instead of nothing.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only the first render seeds the default.
  React.useEffect(() => {
    if (draftUrl.trim() === "")
      actions.updateAvatarUrl(emojiAvatarDataUrl(initials, DEFAULT_SWATCH));
  }, []);

  return (
    <form
      aria-labelledby="op-photo-title"
      className="c3fr-profile-card"
      data-active="true"
      data-testid="onboarding-page-avatar"
      onSubmit={(event) => {
        event.preventDefault();
        if (!busy) actions.submit();
      }}
    >
      <span className="c3fr-eyebrow c3fr-eyebrow-accent">Step 2 of 2</span>
      <div className="c3fr-head">
        <h2
          className="c3fr-title c3fr-title-profile"
          id="op-photo-title"
          tabIndex={-1}
        >
          Next, add a display image
        </h2>
        <p className="c3fr-lede c3fr-lede-card">
          Choose an image or a colour as your avatar.
        </p>
      </div>

      <div className="c3fr-avatar-row">
        {hasImage ? (
          <ProfileAvatar
            avatarUrl={draftUrl}
            className="c3fr-avatar-preview c3fr-avatar-preview-image"
            label={displayName || "You"}
            testId="onboarding-avatar-preview"
          />
        ) : (
          <span
            aria-label="Your avatar preview"
            className="c3fr-avatar-preview"
            data-testid="onboarding-avatar-preview"
            role="img"
            style={{ background: activeColor ?? DEFAULT_SWATCH }}
          >
            {initials}
          </span>
        )}
        <div className="c3fr-avatar-who">
          <strong>{displayName || "You"}</strong>
          <span>This is how you’ll appear in channels and threads.</span>
        </div>
      </div>

      <fieldset className="c3fr-swatches">
        <legend className="c3fr-label">Pick a colour</legend>
        <div>
          {AVATAR_SWATCHES.map((swatch) => (
            <label key={swatch.name}>
              <input
                aria-label={swatch.name}
                checked={activeColor === swatch.color}
                className="c3fr-sr-only"
                disabled={busy}
                name="op-colour"
                onChange={() =>
                  actions.updateAvatarUrl(
                    emojiAvatarDataUrl(initials, swatch.color),
                  )
                }
                type="radio"
              />
              <span
                className="c3fr-swatch"
                data-checked={activeColor === swatch.color ? "true" : undefined}
                style={{ background: swatch.color }}
              >
                {initials}
              </span>
            </label>
          ))}
        </div>
      </fieldset>

      <div className="c3fr-photo-actions">
        <label className="c3fr-photo-button" data-disabled={busy || undefined}>
          <UploadIcon size={17} />
          Upload a photo
          <input
            accept="image/gif,image/jpeg,image/png,image/webp"
            className="c3fr-sr-only"
            data-testid="onboarding-avatar-upload-input"
            disabled={busy}
            onChange={upload.handleFileChange}
            ref={upload.inputRef}
            type="file"
          />
        </label>
        <button
          className="c3fr-photo-button"
          data-testid="onboarding-avatar-take-photo"
          disabled={busy}
          onClick={() => setCameraOpen(true)}
          type="button"
        >
          <CameraIcon size={17} />
          Take a photo
        </button>
      </div>
      {upload.errorMessage ? (
        <FirstRunError>{upload.errorMessage}</FirstRunError>
      ) : null}
      {saveRecovery.errorMessage ? (
        <ErrorBanner isSaving={isSaving} message={saveRecovery.errorMessage} />
      ) : null}

      <div className="c3fr-profile-actions">
        <button
          className="c3fr-pane-primary"
          data-testid="onboarding-next"
          disabled={busy}
          type="submit"
        >
          {isSaving || isUploadingAvatar || isUploading ? "Saving…" : "Next"}
        </button>
        {saveRecovery.canSkipForNow ? (
          <FirstRunLink
            className="c3fr-profile-skip"
            data-testid="onboarding-skip"
            disabled={busy}
            onClick={actions.skipForNow}
          >
            Skip for now
          </FirstRunLink>
        ) : showAlwaysSkip && !saveRecovery.errorMessage ? (
          <FirstRunLink
            className="c3fr-profile-skip"
            data-testid="onboarding-skip"
            disabled={busy}
            onClick={actions.advanceWithoutSaving}
          >
            Skip for now
          </FirstRunLink>
        ) : null}
        {saveRecovery.canAdvanceWithoutSaving ? (
          <FirstRunLink
            className="c3fr-profile-skip"
            data-testid="onboarding-next-without-saving"
            disabled={busy}
            onClick={actions.advanceWithoutSaving}
          >
            Continue without saving
          </FirstRunLink>
        ) : null}
      </div>
      <PhotoCaptureDialog
        onCapture={(file) => {
          setCameraOpen(false);
          void upload.uploadFile(file);
        }}
        onOpenChange={setCameraOpen}
        open={cameraOpen}
      />
    </form>
  );
}
