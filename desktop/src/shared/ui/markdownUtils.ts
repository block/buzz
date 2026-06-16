import * as React from "react";

/**
 * Returns true when a React element is a block-level media wrapper (image or
 * video). The `img` component in `createMarkdownComponents` marks its output
 * with a `data-block-media` prop so we can reliably distinguish media from
 * other custom components (links, mentions, etc.) that also have non-string
 * types in react-markdown v10.
 */
function isBlockMedia(child: React.ReactNode): boolean {
  if (!React.isValidElement(child)) return false;

  const props = child.props as Record<string, unknown>;
  const node = props?.node as { tagName?: unknown } | undefined;

  return props?.["data-block-media"] != null || node?.tagName === "img";
}

/**
 * Classifies an array of React children into media vs non-media buckets.
 * Used by the `p` component to detect image-only paragraphs for gallery
 * rendering.
 *
 * "Image children" = elements marked with `data-block-media` (images/videos).
 * "Non-image children" = everything else, excluding whitespace-only strings
 * and `<br>` elements (injected by remarkBreaks between images).
 */
export function classifyChildren(childArray: React.ReactNode[]): {
  imageChildren: React.ReactNode[];
  nonImageChildren: React.ReactNode[];
} {
  const imageChildren = childArray.filter(isBlockMedia);
  const nonImageChildren = childArray.filter(
    (child) =>
      !isBlockMedia(child) &&
      !(typeof child === "string" && child.trim() === "") &&
      !(React.isValidElement(child) && child.type === "br"),
  );
  return { imageChildren, nonImageChildren };
}

/** Returns true when a paragraph contains 2+ images and no other content. */
export function isImageOnlyParagraph(childArray: React.ReactNode[]): boolean {
  const { imageChildren, nonImageChildren } = classifyChildren(childArray);
  return imageChildren.length >= 2 && nonImageChildren.length === 0;
}

/**
 * Returns true when a paragraph contains any image/video child. The custom
 * `img` renderer always emits block-level markup (lightbox/video wrapper),
 * so any such paragraph must render as `<div>` to avoid invalid `<p><div>`
 * nesting — even when mixed with text or links.
 */
export function hasBlockMedia(childArray: React.ReactNode[]): boolean {
  const { imageChildren } = classifyChildren(childArray);
  return imageChildren.length >= 1;
}

export function shallowArrayEqual(a?: string[], b?: string[]): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

// Display caps for inline message images — must mirror the `max-h-64 max-w-sm`
// (256×384px) Tailwind classes on the rendered <img>.
export const INLINE_IMAGE_MAX_WIDTH = 384;
export const INLINE_IMAGE_MAX_HEIGHT = 256;

/**
 * Compute the exact rendered box for an inline image from its imeta `dim`
 * ("WIDTHxHEIGHT"), scaled to fit the display caps (same fit as `object-contain`
 * within `max-w-sm`/`max-h-64`). Used to reserve the row's height via the <img>
 * width/height attributes BEFORE the bytes load — without it, image rows paint
 * at height 0 then grow when the image arrives, thrashing the virtualized list
 * (every late image re-measures and shifts its neighbors).
 *
 * Returns undefined when `dim` is missing/malformed/non-positive — we can't
 * reserve what we don't know, so the natural load is the correct fallback.
 */
export function reservedImageSize(
  dim: string | undefined,
): { width: number; height: number } | undefined {
  if (!dim) return undefined;
  const match = dim.match(/^(\d+)x(\d+)$/i);
  if (!match) return undefined;
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (
    !Number.isFinite(width) ||
    !Number.isFinite(height) ||
    width <= 0 ||
    height <= 0
  ) {
    return undefined;
  }
  const scale = Math.min(
    1,
    INLINE_IMAGE_MAX_WIDTH / width,
    INLINE_IMAGE_MAX_HEIGHT / height,
  );
  return {
    width: Math.round(width * scale),
    height: Math.round(height * scale),
  };
}
