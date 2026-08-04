import {
  cancelBackgroundMediaUploads,
  useBackgroundMediaUpload,
} from "@/features/messages/lib/backgroundMediaUploadStore";
import { ComposerUploadProgressPill } from "@/features/messages/ui/ComposerUploadProgressPill";

export function ComposerUploadProgressOverlay() {
  const backgroundUpload = useBackgroundMediaUpload();

  return (
    <div className="pointer-events-auto">
      <ComposerUploadProgressPill
        canCancel={backgroundUpload.canCancel}
        isUploading={backgroundUpload.isUploading}
        onCancel={cancelBackgroundMediaUploads}
        phase={backgroundUpload.phase}
        percentage={backgroundUpload.percentage}
      />
    </div>
  );
}
