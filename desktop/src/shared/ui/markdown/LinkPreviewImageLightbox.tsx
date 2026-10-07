import type { ComponentType } from "react";
import { useRef, useState } from "react";

import { cn } from "@/shared/lib/cn";
import type { LinkPreviewImageLightboxProps } from "@/shared/ui/rich-link-preview-attachment";

import {
  type ImageGalleryItem,
  type ImageLightboxBox,
  type ImageLightboxCornerRadii,
  imageLightboxBoxFromRect,
  imageLightboxCornerRadiiFromElement,
  visibleImageGalleryForTrigger,
} from "./imageLightbox";

type ImageZoomOverlayProps = {
  alt: string | undefined;
  galleryIndex?: number;
  galleryItems?: ImageGalleryItem[];
  onClose: () => void;
  onCopy: (src: string | undefined) => void;
  onDownload: (src: string | undefined) => void;
  resolvedSrc: string;
  sourceBox: ImageLightboxBox;
  sourceCornerRadii: ImageLightboxCornerRadii;
  sourceScope?: Element | null;
  src: string | undefined;
};

const ignoreUnavailableImageAction = () => undefined;

export function createLinkPreviewImageLightbox(
  ImageZoomOverlay: ComponentType<ImageZoomOverlayProps>,
): ComponentType<LinkPreviewImageLightboxProps> {
  return function LinkPreviewImageLightbox({ alt, children, className, src }) {
    const [lightboxState, setLightboxState] = useState<{
      galleryIndex: number;
      galleryItems?: ImageGalleryItem[];
      sourceBox: ImageLightboxBox;
      sourceCornerRadii: ImageLightboxCornerRadii;
      sourceScope: Element | null;
    } | null>(null);
    const triggerRef = useRef<HTMLButtonElement | null>(null);

    const openLightbox = () => {
      const trigger = triggerRef.current;
      const image = trigger?.querySelector("img");
      if (!trigger || !image) return;

      const rect = image.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) return;

      const sourceBox = imageLightboxBoxFromRect(rect);
      const sourceCornerRadii = imageLightboxCornerRadiiFromElement(image);
      const sourceScope = trigger.closest("[data-link-preview-list]");
      const dim =
        image.naturalWidth > 0 && image.naturalHeight > 0
          ? `${image.naturalWidth}x${image.naturalHeight}`
          : undefined;
      const gallery = visibleImageGalleryForTrigger(
        trigger,
        {
          alt,
          dim,
          resolvedSrc: src,
          src: undefined,
          thumbnailBox: sourceBox,
          thumbnailCornerRadii: sourceCornerRadii,
        },
        sourceScope,
      );

      setLightboxState({
        galleryIndex: gallery.galleryIndex,
        galleryItems: gallery.galleryItems,
        sourceBox,
        sourceCornerRadii,
        sourceScope,
      });
    };

    return (
      <>
        <button
          aria-label={`Zoom image: ${alt}`}
          className={cn(
            "cursor-zoom-in border-0 p-0 text-left focus:outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50",
            lightboxState && "opacity-0",
            className,
          )}
          data-image-lightbox-alt={alt}
          data-image-lightbox-resolved-src={src}
          data-image-lightbox-trigger=""
          onClick={openLightbox}
          ref={triggerRef}
          type="button"
        >
          {children}
        </button>
        {lightboxState ? (
          <ImageZoomOverlay
            alt={alt}
            galleryIndex={lightboxState.galleryIndex}
            galleryItems={lightboxState.galleryItems}
            onClose={() => setLightboxState(null)}
            onCopy={ignoreUnavailableImageAction}
            onDownload={ignoreUnavailableImageAction}
            resolvedSrc={src}
            sourceBox={lightboxState.sourceBox}
            sourceCornerRadii={lightboxState.sourceCornerRadii}
            sourceScope={lightboxState.sourceScope}
            src={undefined}
          />
        ) : null}
      </>
    );
  };
}
