import * as React from "react";

import { cn } from "@/shared/lib/cn";

const IMAGE_CLASS =
  "absolute inset-0 block h-full w-full rounded-2xl object-contain";

function isSameImageSource(
  left: string | undefined,
  right: string | undefined,
) {
  if (!left || !right) return false;
  try {
    return (
      new URL(left, window.location.href).href ===
      new URL(right, window.location.href).href
    );
  } catch {
    return left === right;
  }
}

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
  const thumbnailSrc = isSameImageSource(thumbSrc, resolvedSrc)
    ? undefined
    : thumbSrc;
  const [loadFullImage, setLoadFullImage] = React.useState(!thumbnailSrc);
  const [fullImageLoaded, setFullImageLoaded] = React.useState(!thumbnailSrc);

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

  const frameStyle = React.useMemo<React.CSSProperties>(() => {
    const scale = Math.min(1, 384 / width, 256 / height);
    return {
      ...style,
      aspectRatio: `${width} / ${height}`,
      height: "auto",
      width: `${Math.max(1, Math.round(width * scale))}px`,
    };
  }, [height, style, width]);

  return (
    <span
      className="relative block max-w-full"
      data-progressive-image-frame=""
      style={frameStyle}
    >
      {thumbnailSrc ? (
        <img
          alt=""
          aria-hidden="true"
          className={IMAGE_CLASS}
          decoding="async"
          height={height}
          loading="lazy"
          ref={setThumbnailRef}
          src={thumbnailSrc}
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
            thumbnailSrc && !fullImageLoaded && "opacity-0",
          )}
          data-spoiler-media-size={showSpoilerSize ? "" : undefined}
          decoding="async"
          height={height}
          loading={thumbnailSrc ? undefined : "lazy"}
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
