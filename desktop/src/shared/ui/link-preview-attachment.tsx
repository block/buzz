import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";
import { useLinkPreviewStyle } from "@/shared/lib/linkPreviewStylePreference";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useMediaProxyPort } from "@/shared/lib/useMediaProxyPort";
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
  useMediaProxyPort();
  const renderedPreview = {
    ...preview,
    faviconDataUrl: preview.faviconDataUrl
      ? rewriteRelayUrl(preview.faviconDataUrl)
      : null,
    imageDataUrl: preview.imageDataUrl
      ? rewriteRelayUrl(preview.imageDataUrl)
      : null,
  };
  const style = useLinkPreviewStyle();
  if (style === "rich") {
    return (
      <RichLinkPreviewAttachment
        className={className}
        ImageLightbox={ImageLightbox}
        onOpen={onOpen}
        onRemove={onRemove}
        preview={renderedPreview}
        showControls={showControls}
      />
    );
  }

  return (
    <CompactLinkPreviewAttachment
      className={className}
      onOpen={onOpen}
      onRemove={onRemove}
      preview={renderedPreview}
      showControls={showControls}
    />
  );
}
