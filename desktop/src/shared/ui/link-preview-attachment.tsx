import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";
import { useLinkPreviewStyle } from "@/shared/lib/linkPreviewStylePreference";
import { CompactLinkPreviewAttachment } from "@/shared/ui/compact-link-preview-attachment";
import {
  type LinkPreviewImageLightboxComponent,
  RichLinkPreviewAttachment,
} from "@/shared/ui/rich-link-preview-attachment";

export function LinkPreviewAttachment({
  className,
  ImageLightbox,
  onOpen,
  onRemove,
  preview,
  showControls,
}: {
  className?: string;
  ImageLightbox: LinkPreviewImageLightboxComponent;
  onOpen?: () => void;
  onRemove?: () => void;
  preview: ResolvedLinkPreview;
  showControls?: boolean;
}) {
  const style = useLinkPreviewStyle();
  if (style === "rich") {
    return (
      <RichLinkPreviewAttachment
        className={className}
        ImageLightbox={ImageLightbox}
        onOpen={onOpen}
        onRemove={onRemove}
        preview={preview}
        showControls={showControls}
      />
    );
  }

  return (
    <CompactLinkPreviewAttachment
      className={className}
      onOpen={onOpen}
      onRemove={onRemove}
      preview={preview}
      showControls={showControls}
    />
  );
}
