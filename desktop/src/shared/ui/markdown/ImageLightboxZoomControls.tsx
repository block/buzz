import type { CSSProperties } from "react";
import { ZoomIn, ZoomOut } from "lucide-react";

import {
  IMAGE_LIGHTBOX_MAX_ZOOM,
  IMAGE_LIGHTBOX_MIN_ZOOM,
  IMAGE_LIGHTBOX_ZOOM_STEP,
} from "./imageLightbox";

type ImageLightboxZoomControlsProps = {
  markControlGesture: () => void;
  setClampedZoom: (nextZoom: number) => void;
  setIsAdjustingZoom: (isAdjusting: boolean) => void;
  updateZoom: (updater: (currentZoom: number) => number) => void;
  zoom: number;
};

const ZOOM_BUTTON_CLASS_NAME =
  "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors hover:bg-muted-foreground/10 hover:text-foreground outline-hidden focus-visible:ring-2 focus-visible:ring-ring/70 disabled:pointer-events-none disabled:opacity-45";

/** Interactive zoom controls for the image lightbox toolbar. */
export function ImageLightboxZoomControls({
  markControlGesture,
  setClampedZoom,
  setIsAdjustingZoom,
  updateZoom,
  zoom,
}: ImageLightboxZoomControlsProps) {
  const zoomFillPercent =
    ((zoom - IMAGE_LIGHTBOX_MIN_ZOOM) /
      (IMAGE_LIGHTBOX_MAX_ZOOM - IMAGE_LIGHTBOX_MIN_ZOOM)) *
    100;

  return (
    <>
      <button
        aria-label="Zoom out"
        className={ZOOM_BUTTON_CLASS_NAME}
        disabled={zoom <= IMAGE_LIGHTBOX_MIN_ZOOM}
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          markControlGesture();
          updateZoom((currentZoom) => currentZoom - IMAGE_LIGHTBOX_ZOOM_STEP);
        }}
      >
        <ZoomOut aria-hidden="true" className="h-4 w-4 opacity-80" />
      </button>
      <input
        aria-label="Image zoom"
        className="image-zoom-slider h-3 w-32 cursor-pointer sm:w-44"
        max={IMAGE_LIGHTBOX_MAX_ZOOM}
        min={IMAGE_LIGHTBOX_MIN_ZOOM}
        step={IMAGE_LIGHTBOX_ZOOM_STEP}
        style={{ "--image-zoom-fill": `${zoomFillPercent}%` } as CSSProperties}
        type="range"
        value={zoom}
        onBlur={() => setIsAdjustingZoom(false)}
        onChange={(event) => {
          markControlGesture();
          setClampedZoom(Number(event.target.value));
        }}
        onPointerCancel={() => setIsAdjustingZoom(false)}
        onPointerDown={() => {
          markControlGesture();
          setIsAdjustingZoom(true);
        }}
        onPointerUp={() => {
          markControlGesture();
          setIsAdjustingZoom(false);
        }}
      />
      <button
        aria-label="Zoom in"
        className={ZOOM_BUTTON_CLASS_NAME}
        disabled={zoom >= IMAGE_LIGHTBOX_MAX_ZOOM}
        type="button"
        onClick={(event) => {
          event.stopPropagation();
          markControlGesture();
          updateZoom((currentZoom) => currentZoom + IMAGE_LIGHTBOX_ZOOM_STEP);
        }}
      >
        <ZoomIn aria-hidden="true" className="h-4 w-4 opacity-80" />
      </button>
      <span className="min-w-10 text-right text-xs font-medium tabular-nums text-muted-foreground">
        {Math.round(zoom * 100)}%
      </span>
    </>
  );
}
