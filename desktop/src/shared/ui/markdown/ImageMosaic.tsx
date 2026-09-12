import React from "react";
import { cn } from "@/shared/lib/cn";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

/** Grid layout for messages that contain multiple standalone images. */
export function ImageMosaic({ children }: { children: React.ReactNode[] }) {
  const mosaicRef = React.useRef<HTMLDivElement | null>(null);
  const isTriptych = children.length === 3;
  const hasOddTail = children.length > 3 && children.length % 2 === 1;
  useSmoothCorners(mosaicRef);

  return (
    <div
      className={cn(
        "mt-1 grid w-full min-w-0 max-w-lg grid-cols-2 gap-1.5 overflow-hidden rounded-2xl [&_br]:hidden [&_[data-block-media]]:min-h-0 [&_[data-block-media]]:max-w-none [&_[data-block-media]]:overflow-hidden [&_[data-block-media]>button]:m-0 [&_[data-block-media]>button]:h-full [&_[data-block-media]>button]:w-full [&_[data-block-media]>button]:max-w-none [&_[data-block-media]>button]:rounded-none [&_[data-block-media]_[data-progressive-image-frame]]:!h-full [&_[data-block-media]_[data-progressive-image-frame]]:!w-full [&_[data-block-media]_img]:!h-full [&_[data-block-media]_img]:!max-h-none [&_[data-block-media]_img]:!w-full [&_[data-block-media]_img]:!max-w-none [&_[data-block-media]_img]:rounded-none [&_[data-block-media]_img]:object-cover",
        isTriptych
          ? "h-80 grid-rows-2 [&_[data-block-media]]:h-auto [&_[data-block-media]:first-child]:row-span-2"
          : "[&_[data-block-media]]:h-48",
        hasOddTail && "[&_[data-block-media]:last-child]:col-span-2",
      )}
      data-image-mosaic=""
      data-image-mosaic-count={children.length}
      ref={mosaicRef}
    >
      {children}
    </div>
  );
}
