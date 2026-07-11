import * as React from "react";

import { cn } from "@/shared/lib/cn";

const IMAGE_CLASS =
  "col-start-1 row-start-1 block h-auto max-h-64 max-w-[min(24rem,100%)] rounded-2xl object-contain";

type ProgressiveImageProps = {
  alt: string | undefined;
  fullImageRef: React.RefObject<HTMLImageElement | null>;
  height: number;
  onFullLoad: (image: HTMLImageElement) => void;
  onThumbnailLoad: (image: HTMLImageElement) => void;
  resolvedSrc: string | undefined;
  showSpoilerSize: boolean;
  style: React.CSSProperties | undefined;
  thumbnailRef: React.RefObject<HTMLImageElement | null>;
  thumbSrc: string | undefined;
  width: number;
};

export function ProgressiveImage({
  alt,
  fullImageRef,
  height,
  onFullLoad,
  onThumbnailLoad,
  resolvedSrc,
  showSpoilerSize,
  style,
  thumbnailRef,
  thumbSrc,
  width,
}: ProgressiveImageProps) {
  const [loadFullImage, setLoadFullImage] = React.useState(!thumbSrc);
  const [fullImageLoaded, setFullImageLoaded] = React.useState(!thumbSrc);

  const handleFullLoad = React.useCallback(
    async (image: HTMLImageElement) => {
      onFullLoad(image);
      try {
        await image.decode();
      } catch {
        // The load event still proves the image is displayable.
      }
      setFullImageLoaded(true);
    },
    [onFullLoad],
  );

  const setFullImageRef = React.useCallback(
    (image: HTMLImageElement | null) => {
      fullImageRef.current = image;
      if (image?.complete) void handleFullLoad(image);
    },
    [fullImageRef, handleFullLoad],
  );

  const setThumbnailRef = React.useCallback(
    (image: HTMLImageElement | null) => {
      thumbnailRef.current = image;
      if (image && !fullImageRef.current) fullImageRef.current = image;
    },
    [fullImageRef, thumbnailRef],
  );

  return (
    <span className="grid max-w-full">
      {thumbSrc ? (
        <img
          alt=""
          aria-hidden="true"
          className={IMAGE_CLASS}
          decoding="async"
          height={height}
          loading="lazy"
          ref={setThumbnailRef}
          src={thumbSrc}
          style={style}
          width={width}
          onError={() => setLoadFullImage(true)}
          onLoad={(event) => {
            onThumbnailLoad(event.currentTarget);
            setLoadFullImage(true);
          }}
        />
      ) : null}
      {loadFullImage ? (
        <img
          alt={alt}
          className={cn(
            IMAGE_CLASS,
            "transition-opacity duration-200 motion-reduce:transition-none",
            thumbSrc && !fullImageLoaded && "opacity-0",
          )}
          data-spoiler-media-size={showSpoilerSize ? "" : undefined}
          decoding="async"
          height={height}
          loading={thumbSrc ? undefined : "lazy"}
          ref={setFullImageRef}
          src={resolvedSrc}
          style={style}
          width={width}
          onLoad={(event) => void handleFullLoad(event.currentTarget)}
        />
      ) : null}
    </span>
  );
}
