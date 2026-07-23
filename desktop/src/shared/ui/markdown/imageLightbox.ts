import type { CSSProperties } from "react";

import { MESSAGE_MARKDOWN_CLASS } from "@/shared/ui/mentionChip";

import {
  dimensionsFromDim,
  getDecodedImageDimensions,
  isInsideHiddenSpoiler,
} from "./utils";

export type ImageLightboxBox = {
  height: number;
  left: number;
  top: number;
  width: number;
};

export type ImageLightboxCornerRadii = {
  bottomLeft: string;
  bottomRight: string;
  topLeft: string;
  topRight: string;
};

type ImageLightboxThumbnailTarget = {
  box: ImageLightboxBox;
  cornerRadii: ImageLightboxCornerRadii;
};

export type ImageGalleryDirection = "forward" | "backward";

export type ImageGalleryItem = {
  alt: string | undefined;
  dim?: string;
  resolvedSrc: string;
  src: string | undefined;
  thumbnailBox?: ImageLightboxBox;
  thumbnailCornerRadii?: ImageLightboxCornerRadii;
};

export const IMAGE_LIGHTBOX_ENTER_MS = 260;
export const IMAGE_LIGHTBOX_EXIT_MS = 170;
export const IMAGE_LIGHTBOX_FADE_ENTER_MS = 180;
export const IMAGE_LIGHTBOX_FADE_EXIT_MS = 90;
export const IMAGE_LIGHTBOX_GALLERY_SLIDE_MS = 280;
export const IMAGE_LIGHTBOX_GALLERY_SLIDE_DISTANCE_PX = 48;
export const IMAGE_LIGHTBOX_GALLERY_BLUR_PX = 4;
export const IMAGE_LIGHTBOX_REDUCED_MOTION_MS = 100;
export const IMAGE_LIGHTBOX_ZOOM_TRANSITION_MS = 80;
export const IMAGE_LIGHTBOX_BASE_VIEWPORT_RATIO = 0.8;
export const IMAGE_LIGHTBOX_CONTROL_SUPPRESS_CLOSE_MS = 450;
export const IMAGE_LIGHTBOX_TRACKPAD_ZOOM_IDLE_MS = 120;
export const IMAGE_LIGHTBOX_WHEEL_ZOOM_SPEED = 0.002;
export const IMAGE_LIGHTBOX_WHEEL_ZOOM_MAX_DELTA = 0.2;
export const IMAGE_LIGHTBOX_MIN_ZOOM = 1;
export const IMAGE_LIGHTBOX_MAX_ZOOM = 3;
export const IMAGE_LIGHTBOX_ZOOM_STEP = 0.05;
export const IMAGE_LIGHTBOX_EASE_OUT = "cubic-bezier(0.23, 1, 0.32, 1)";
export const IMAGE_LIGHTBOX_EASE_IN_OUT = "cubic-bezier(0.77, 0, 0.175, 1)";
export const IMAGE_LIGHTBOX_EXPANDED_CORNER_RADIUS = "1rem";
export const IMAGE_LIGHTBOX_GALLERY_EASE: [number, number, number, number] = [
  0.22, 1, 0.36, 1,
];
export const IMAGE_LIGHTBOX_MARKDOWN_SCOPE_SELECTOR = `.${MESSAGE_MARKDOWN_CLASS}`;

export function imageLightboxBoxFromRect(rect: DOMRect): ImageLightboxBox {
  return {
    height: rect.height,
    left: rect.left,
    top: rect.top,
    width: rect.width,
  };
}

export function imageLightboxTargetBox(
  sourceBox: ImageLightboxBox,
): ImageLightboxBox {
  const viewportWidth = window.innerWidth;
  const viewportHeight = window.innerHeight;
  const horizontalPadding = Math.min(80, Math.max(16, viewportWidth * 0.0625));
  const verticalPadding = Math.min(24, Math.max(16, viewportHeight * 0.033));
  const maxWidth = Math.max(
    1,
    Math.min(
      viewportWidth - horizontalPadding * 2,
      viewportWidth * IMAGE_LIGHTBOX_BASE_VIEWPORT_RATIO,
    ),
  );
  const maxHeight = Math.max(
    1,
    Math.min(
      viewportHeight - verticalPadding * 2,
      viewportHeight * IMAGE_LIGHTBOX_BASE_VIEWPORT_RATIO,
    ),
  );
  const scale = Math.min(
    maxWidth / sourceBox.width,
    maxHeight / sourceBox.height,
  );
  const width = Math.max(1, sourceBox.width * scale);
  const height = Math.max(1, sourceBox.height * scale);

  return {
    height,
    left: (viewportWidth - width) / 2,
    top: (viewportHeight - height) / 2,
    width,
  };
}

export function imageLightboxStyle(box: ImageLightboxBox): CSSProperties {
  return {
    height: `${box.height}px`,
    left: `${box.left}px`,
    top: `${box.top}px`,
    width: `${box.width}px`,
  };
}

export function clampImageLightboxZoom(value: number): number {
  return Math.min(
    IMAGE_LIGHTBOX_MAX_ZOOM,
    Math.max(IMAGE_LIGHTBOX_MIN_ZOOM, value),
  );
}

export function normalizedWheelDeltaY(event: WheelEvent): number {
  if (event.deltaMode === WheelEvent.DOM_DELTA_LINE) {
    return event.deltaY * 16;
  }

  if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE) {
    return event.deltaY * window.innerHeight;
  }

  return event.deltaY;
}

export function imageLightboxTransform(
  sourceBox: ImageLightboxBox,
  targetBox: ImageLightboxBox,
): string {
  const scaleX = targetBox.width / Math.max(1, sourceBox.width);
  const scaleY = targetBox.height / Math.max(1, sourceBox.height);
  const translateX = targetBox.left - sourceBox.left;
  const translateY = targetBox.top - sourceBox.top;

  return `translate3d(${translateX}px, ${translateY}px, 0) scale(${scaleX}, ${scaleY})`;
}

export function imageLightboxZoomBox(
  targetBox: ImageLightboxBox,
  zoom: number,
): ImageLightboxBox {
  const width = targetBox.width * zoom;
  const height = targetBox.height * zoom;

  return {
    height,
    left: targetBox.left + (targetBox.width - width) / 2,
    top: targetBox.top + (targetBox.height - height) / 2,
    width,
  };
}

export function imageLightboxBasisBoxForItem(
  item: ImageGalleryItem,
  fallbackBox: ImageLightboxBox,
): ImageLightboxBox {
  const dimensions =
    dimensionsFromDim(item.dim) ?? getDecodedImageDimensions(item.resolvedSrc);
  if (!dimensions) {
    return item.thumbnailBox ?? fallbackBox;
  }

  return {
    ...fallbackBox,
    height: dimensions.height,
    width: dimensions.width,
  };
}

export function imageLightboxCornerRadiiFromElement(
  element: Element,
): ImageLightboxCornerRadii {
  const style = window.getComputedStyle(element);
  const cornerRadii = {
    bottomLeft: style.borderBottomLeftRadius,
    bottomRight: style.borderBottomRightRadius,
    topLeft: style.borderTopLeftRadius,
    topRight: style.borderTopRightRadius,
  };
  const mosaic = element.closest<HTMLElement>("[data-image-mosaic]");
  const geometryElement =
    element.closest<HTMLElement>("[data-progressive-image-frame]") ?? element;
  if (!mosaic || mosaic === element) {
    return cornerRadii;
  }

  // Mosaic tiles themselves are square. Their visible outer corners come from
  // the gallery container's clip, so preserve only the container corners that
  // this tile actually touches when the overlay returns.
  const elementRect = geometryElement.getBoundingClientRect();
  const mosaicRect = mosaic.getBoundingClientRect();
  const mosaicStyle = window.getComputedStyle(mosaic);
  const touches = (first: number, second: number) =>
    Math.abs(first - second) < 1;
  const touchesTop = touches(elementRect.top, mosaicRect.top);
  const touchesRight = touches(elementRect.right, mosaicRect.right);
  const touchesBottom = touches(elementRect.bottom, mosaicRect.bottom);
  const touchesLeft = touches(elementRect.left, mosaicRect.left);

  return {
    bottomLeft:
      touchesBottom && touchesLeft
        ? mosaicStyle.borderBottomLeftRadius
        : cornerRadii.bottomLeft,
    bottomRight:
      touchesBottom && touchesRight
        ? mosaicStyle.borderBottomRightRadius
        : cornerRadii.bottomRight,
    topLeft:
      touchesTop && touchesLeft
        ? mosaicStyle.borderTopLeftRadius
        : cornerRadii.topLeft,
    topRight:
      touchesTop && touchesRight
        ? mosaicStyle.borderTopRightRadius
        : cornerRadii.topRight,
  };
}

export function imageLightboxCornerRadiiStyle(
  cornerRadii: ImageLightboxCornerRadii,
): CSSProperties {
  return {
    borderBottomLeftRadius: cornerRadii.bottomLeft,
    borderBottomRightRadius: cornerRadii.bottomRight,
    borderTopLeftRadius: cornerRadii.topLeft,
    borderTopRightRadius: cornerRadii.topRight,
  };
}

export function imageLightboxExpandedCornerRadii(): ImageLightboxCornerRadii {
  return {
    bottomLeft: IMAGE_LIGHTBOX_EXPANDED_CORNER_RADIUS,
    bottomRight: IMAGE_LIGHTBOX_EXPANDED_CORNER_RADIUS,
    topLeft: IMAGE_LIGHTBOX_EXPANDED_CORNER_RADIUS,
    topRight: IMAGE_LIGHTBOX_EXPANDED_CORNER_RADIUS,
  };
}

function imageLightboxThumbnailTargetForItem(
  item: ImageGalleryItem,
  sourceScope: Element | null | undefined,
): ImageLightboxThumbnailTarget | null {
  const root = sourceScope?.isConnected ? sourceScope : document.body;
  const triggers = Array.from(
    root.querySelectorAll<HTMLElement>("[data-image-lightbox-trigger]"),
  );

  for (const trigger of triggers) {
    const isCurrentItem =
      trigger.dataset.imageLightboxResolvedSrc === item.resolvedSrc ||
      (item.src != null && trigger.dataset.imageLightboxSrc === item.src);
    if (!isCurrentItem) {
      continue;
    }

    const image = trigger.querySelector("img");
    const target = image ?? trigger;
    const rect = target.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      return {
        box: imageLightboxBoxFromRect(rect),
        cornerRadii: imageLightboxCornerRadiiFromElement(target),
      };
    }
  }

  return null;
}

export function imageLightboxReturnTargetForItem(
  item: ImageGalleryItem,
  fallbackBox: ImageLightboxBox,
  fallbackCornerRadii: ImageLightboxCornerRadii,
  sourceScope: Element | null | undefined,
): ImageLightboxThumbnailTarget {
  const currentTarget = imageLightboxThumbnailTargetForItem(item, sourceScope);
  if (currentTarget) {
    return currentTarget;
  }

  return {
    box: item.thumbnailBox ?? fallbackBox,
    cornerRadii: item.thumbnailCornerRadii ?? fallbackCornerRadii,
  };
}

export function imageLightboxSourceScopeForTrigger(
  trigger: HTMLElement,
): Element | null {
  return (
    trigger.closest(IMAGE_LIGHTBOX_MARKDOWN_SCOPE_SELECTOR) ??
    trigger.closest("[data-testid='message-row']")
  );
}

function imageGalleryItemFromTrigger(
  trigger: HTMLElement,
  thumbnail?: ImageLightboxThumbnailTarget,
): ImageGalleryItem | null {
  const resolvedSrc = trigger.dataset.imageLightboxResolvedSrc;
  if (!resolvedSrc) {
    return null;
  }

  return {
    alt: trigger.dataset.imageLightboxAlt || undefined,
    dim: trigger.dataset.imageLightboxDim || undefined,
    resolvedSrc,
    src: trigger.dataset.imageLightboxSrc || undefined,
    thumbnailBox: thumbnail?.box,
    thumbnailCornerRadii: thumbnail?.cornerRadii,
  };
}

function isVisibleImageLightboxTrigger(trigger: HTMLElement): boolean {
  if (isInsideHiddenSpoiler(trigger)) {
    return false;
  }

  const image = trigger.querySelector("img");
  for (const element of [trigger, image]) {
    if (!element) {
      continue;
    }

    const style = window.getComputedStyle(element);
    if (
      style.display === "none" ||
      style.visibility === "hidden" ||
      Number(style.opacity) === 0
    ) {
      return false;
    }
  }

  return true;
}

export function visibleImageGalleryForTrigger(
  trigger: HTMLElement,
  fallbackItem: ImageGalleryItem,
  sourceScope: Element | null | undefined,
): { galleryIndex: number; galleryItems?: ImageGalleryItem[] } {
  const root = sourceScope?.isConnected ? sourceScope : null;
  const triggers = root
    ? Array.from(
        root.querySelectorAll<HTMLElement>("[data-image-lightbox-trigger]"),
      )
    : [trigger];
  const galleryItems: ImageGalleryItem[] = [];
  let galleryIndex = 0;
  let foundCurrentTrigger = false;

  for (const candidate of triggers) {
    if (!isVisibleImageLightboxTrigger(candidate)) {
      continue;
    }

    const image = candidate.querySelector("img");
    const target = image ?? candidate;
    const rect = target.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) {
      continue;
    }

    const thumbnail = {
      box: imageLightboxBoxFromRect(rect),
      cornerRadii: imageLightboxCornerRadiiFromElement(target),
    };
    const item = imageGalleryItemFromTrigger(candidate, thumbnail);
    if (!item) {
      continue;
    }

    if (candidate === trigger) {
      galleryIndex = galleryItems.length;
      foundCurrentTrigger = true;
    }
    galleryItems.push(item);
  }

  if (!foundCurrentTrigger) {
    galleryItems.unshift(fallbackItem);
    galleryIndex = 0;
  }

  return {
    galleryIndex,
    galleryItems: galleryItems.length > 1 ? galleryItems : undefined,
  };
}
