import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";
import {
  type LinkPreviewStyle,
  useLinkPreviewStyle,
} from "@/shared/lib/linkPreviewStylePreference";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { useMediaProxyPort } from "@/shared/lib/useMediaProxyPort";
import { CompactLinkPreviewAttachment } from "@/shared/ui/compact-link-preview-attachment";
import {
  type LinkPreviewImageLightboxComponent,
  RichLinkPreviewAttachment,
} from "@/shared/ui/rich-link-preview-attachment";

type LinkPreviewAttachmentProps = {
  className?: string;
  ImageLightbox: LinkPreviewImageLightboxComponent;
  onOpen?: () => void;
  onRemove?: () => void;
  preview: ResolvedLinkPreview;
  showControls?: boolean;
  showExpandControl?: boolean;
};

/** Renders a link preview with an explicit presentation style. */
export function LinkPreviewAttachmentPresentation({
  className,
  ImageLightbox,
  onOpen,
  onRemove,
  preview,
  showControls,
  showExpandControl,
  style,
}: LinkPreviewAttachmentProps & {
  style: LinkPreviewStyle;
}) {
  if (style === "rich") {
    return (
      <RichLinkPreviewAttachment
        className={className}
        ImageLightbox={ImageLightbox}
        onOpen={onOpen}
        onRemove={onRemove}
        preview={preview}
        showControls={showControls}
        showExpandControl={showExpandControl}
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

/** Renders a link preview using the user's saved presentation preference. */
export function LinkPreviewAttachment({
  preview,
  ...props
}: LinkPreviewAttachmentProps) {
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

  return (
    <LinkPreviewAttachmentPresentation
      {...props}
      preview={renderedPreview}
      style={style}
    />
  );
}
