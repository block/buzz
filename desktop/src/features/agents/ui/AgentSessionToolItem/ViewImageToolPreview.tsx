import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";

import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { isInlineImageData } from "../agentSessionUtils";

export function ViewImageToolPreview({
  src,
  title,
}: {
  src: string;
  title: string | null;
}) {
  const [lightboxOpen, setLightboxOpen] = React.useState(false);
  const [imageFailed, setImageFailed] = React.useState(false);
  const resolvedSrc = React.useMemo(() => resolveImageSrc(src), [src]);
  const alt = title ?? "Viewed image";

  if (imageFailed) {
    return null;
  }

  return (
    <>
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: opens lightbox on click */}
      <img
        alt={alt}
        className="ml-1.5 block max-h-64 max-w-[min(24rem,calc(100%-0.375rem))] cursor-pointer rounded-lg object-contain"
        decoding="async"
        loading="lazy"
        onClick={() => setLightboxOpen(true)}
        onError={() => setImageFailed(true)}
        src={resolvedSrc}
        title={title ?? undefined}
      />
      <ImageLightbox
        alt={alt}
        onOpenChange={setLightboxOpen}
        open={lightboxOpen}
        src={resolvedSrc}
      />
    </>
  );
}

function ImageLightbox({
  alt,
  onOpenChange,
  open,
  src,
}: {
  alt: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  src: string;
}) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content
          className="fixed inset-0 z-50 flex items-center justify-center p-8"
          onInteractOutside={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
        >
          <DialogPrimitive.Title className="sr-only">
            {alt}
          </DialogPrimitive.Title>
          <DialogPrimitive.Description className="sr-only">
            Full-size image preview. Press Escape or click outside the image to
            close.
          </DialogPrimitive.Description>
          <DialogPrimitive.Close
            aria-label="Close lightbox"
            className="absolute inset-0 cursor-default"
          />
          <img
            alt={alt}
            className="relative max-h-[90vh] max-w-[90vw] rounded-lg object-contain"
            src={src}
          />
          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-full bg-black/50 p-2 text-white/80 transition-colors hover:bg-black/70 hover:text-white focus:outline-hidden focus:ring-2 focus:ring-white/30">
            <svg
              aria-hidden="true"
              fill="none"
              height="20"
              viewBox="0 0 24 24"
              width="20"
              xmlns="http://www.w3.org/2000/svg"
            >
              <path
                d="M18 6L6 18M6 6l12 12"
                stroke="currentColor"
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth="2"
              />
            </svg>
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function resolveImageSrc(source: string): string {
  return isInlineImageData(source) ? source : rewriteRelayUrl(source);
}
