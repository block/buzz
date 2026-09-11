import type { useMediaUpload } from "@/features/messages/lib/useMediaUpload";

/** Display the upload failure without losing the composer's retry controls. */
export function ComposerUploadError({
  uploadState,
  onDismiss,
}: {
  uploadState: ReturnType<typeof useMediaUpload>["uploadState"];
  onDismiss: () => void;
}) {
  if (uploadState.status !== "error") return null;
  return (
    <div className="mb-2 rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
      Upload failed: {uploadState.message}
      <button className="ml-2 underline" onClick={onDismiss} type="button">
        Dismiss
      </button>
    </div>
  );
}
