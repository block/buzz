import { uiT, useUiLanguage } from "@/shared/i18n/uiTranslation";
// Buzz source localisation: authored UI strings only; user values stay unchanged.
import {
  cancelBackgroundMediaUploads,
  useBackgroundMediaUpload,
} from "@/features/messages/lib/backgroundMediaUploadStore";
import {
  skipBackgroundLinkPreviews,
  useBackgroundLinkPreviewPreparation,
} from "@/features/messages/lib/linkPreviewPreparationStore";
import { ComposerUploadProgressPill } from "@/features/messages/ui/ComposerUploadProgressPill";

export function ComposerUploadProgressOverlay() {
  useUiLanguage();

  const backgroundUpload = useBackgroundMediaUpload();
  const linkPreviews = useBackgroundLinkPreviewPreparation();

  return (
    <div className="pointer-events-auto">
      {linkPreviews.isPreparing ? (
        <ComposerUploadProgressPill
          actionLabel={uiT("Skip")}
          canCancel={linkPreviews.canSkip}
          isUploading
          onCancel={skipBackgroundLinkPreviews}
          phase="preparing"
          phaseLabel={uiT("Preparing link preview")}
          percentage={0}
        />
      ) : (
        <ComposerUploadProgressPill
          canCancel={backgroundUpload.canCancel}
          isUploading={backgroundUpload.isUploading}
          onCancel={cancelBackgroundMediaUploads}
          phase={backgroundUpload.phase}
          percentage={backgroundUpload.percentage}
        />
      )}
    </div>
  );
}
