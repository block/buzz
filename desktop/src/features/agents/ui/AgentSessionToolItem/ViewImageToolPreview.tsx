// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { useUiLanguage } from "@/shared/i18n/uiTranslation";
import * as React from "react";

import { SimpleImageLightbox } from "@/shared/ui/SimpleImageLightbox";
import { resolveToolImageSrc } from "../agentSessionUtils";

export function ViewImageToolPreview({
  src,
  title,
}: {
  src: string;
  title: string | null;
}) {
  useUiLanguage();

  const [lightboxOpen, setLightboxOpen] = React.useState(false);
  const [imageFailed, setImageFailed] = React.useState(false);
  const resolvedSrc = React.useMemo(() => resolveToolImageSrc(src), [src]);
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
      <SimpleImageLightbox
        alt={alt}
        onOpenChange={setLightboxOpen}
        open={lightboxOpen}
        src={resolvedSrc}
      />
    </>
  );
}
