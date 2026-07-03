import * as React from "react";
import { defaultUrlTransform } from "react-markdown";

import { isMessageLink } from "@/features/messages/lib/messageLink";

export function useStableArray<T>(arr: T[]): T[] {
  const ref = React.useRef(arr);
  if (
    arr.length !== ref.current.length ||
    arr.some((item, i) => item !== ref.current[i])
  ) {
    ref.current = arr;
  }
  return ref.current;
}

export function aspectRatioFromDim(dim?: string): number | undefined {
  if (!dim) return undefined;
  const match = dim.match(/^(\d+)x(\d+)$/i);
  if (!match) return undefined;
  const width = Number(match[1]);
  const height = Number(match[2]);
  if (!Number.isFinite(width) || !Number.isFinite(height) || height <= 0) {
    return undefined;
  }
  return width / height;
}

/**
 * Parse a NIP-92 `dim` value ("WxH") into intrinsic pixel dimensions. Used to
 * stamp explicit `width`/`height` attributes on inline images so the browser
 * reserves aspect-ratio-correct layout space *before* the image decodes. This
 * is what keeps the timeline from jumping when a tall image loads late — the
 * row's height is known at first paint instead of growing from ~0 on load.
 */
export function dimensionsFromDim(
  dim?: string,
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
  return { width, height };
}

// Natural pixel sizes of images that arrived without a NIP-92 `dim` tag,
// keyed by resolved URL and learned from the first decode. A re-render or
// scrollback of the same URL then reserves correct space immediately instead
// of growing the row from ~0 on decode.
const decodedImageDimensions = new Map<
  string,
  { height: number; width: number }
>();

export function rememberDecodedImageDimensions(
  url: string | undefined,
  width: number,
  height: number,
): void {
  if (!url || !Number.isFinite(width) || !Number.isFinite(height)) return;
  if (width <= 0 || height <= 0) return;
  decodedImageDimensions.set(url, { height, width });
}

export function getDecodedImageDimensions(
  url: string | undefined,
): { width: number; height: number } | undefined {
  return url ? decodedImageDimensions.get(url) : undefined;
}

// Fixed box for a dim-less image whose real size isn't known yet — reserves a
// stable height (matching the inline max-h-64 cap) so a late decode letterboxes
// inside it instead of growing the row. Width is the inline display cap.
const DEFAULT_IMAGE_RESERVE = { height: 256, width: 384 } as const;

/**
 * Decide the layout box to reserve for an inline image before it decodes.
 *
 * Prefer the NIP-92 `dim`, else the size learned from a prior decode of this
 * URL — both let the row settle at the image's true height with no shift. A
 * first-ever dim-less image has no known size, so it reserves a fixed-height
 * box (caller letterboxes via object-contain) that does NOT change when the
 * bytes arrive; the decoded size is cached for the next view.
 */
function resolveImageReserveBox(
  dim: string | undefined,
  resolvedSrc: string | undefined,
): {
  intrinsicDimensions: { width: number; height: number };
  useFixedReserveBox: boolean;
} {
  const known =
    dimensionsFromDim(dim) ?? getDecodedImageDimensions(resolvedSrc);
  return {
    intrinsicDimensions: known ?? DEFAULT_IMAGE_RESERVE,
    useFixedReserveBox: !known,
  };
}

/**
 * Resolve the image reserve box once per mount. `resolveImageReserveBox` is
 * pure in `(dim, resolvedSrc)` and those are stable for a mounted image, so a
 * ref-freeze keeps the box from flipping when the decoded size is cached
 * mid-view — which would re-introduce the shift the reservation prevents.
 */
export function useFrozenImageReserve(
  dim: string | undefined,
  resolvedSrc: string | undefined,
): ReturnType<typeof resolveImageReserveBox> {
  const key = `${dim ?? ""}\u0000${resolvedSrc ?? ""}`;
  const ref = React.useRef<{
    key: string;
    reserve: ReturnType<typeof resolveImageReserveBox>;
  } | null>(null);
  if (!ref.current || ref.current.key !== key) {
    ref.current = { key, reserve: resolveImageReserveBox(dim, resolvedSrc) };
  }
  return ref.current.reserve;
}

/**
 * Inline style for the message image element. A revealed spoiler pins the
 * decoded size; a dim-less reserve pins a fixed-height box so a late decode
 * letterboxes inside it; otherwise the width/height attributes drive layout.
 */
export function imageReserveStyle(args: {
  hiddenSpoilerMediaSize: { height: number; width: number } | null;
  intrinsicDimensions: { height: number; width: number };
  useFixedReserveBox: boolean;
}): React.CSSProperties | undefined {
  const { hiddenSpoilerMediaSize, intrinsicDimensions, useFixedReserveBox } =
    args;
  if (hiddenSpoilerMediaSize) {
    const ratio = `${hiddenSpoilerMediaSize.width} / ${hiddenSpoilerMediaSize.height}`;
    return {
      "--buzz-spoiler-media-aspect-ratio": ratio,
      "--buzz-spoiler-media-width": `${hiddenSpoilerMediaSize.width}px`,
      aspectRatio: ratio,
      height: "auto",
      width: `${hiddenSpoilerMediaSize.width}px`,
    } as React.CSSProperties;
  }
  if (useFixedReserveBox) {
    return {
      height: `${intrinsicDimensions.height}px`,
      width: "min(24rem, 100%)",
    };
  }
  return undefined;
}

export function isInsideHiddenSpoiler(element: Element): boolean {
  return (
    element.closest('.buzz-spoiler[data-spoiler][data-revealed="false"]') !==
    null
  );
}

/**
 * `urlTransform` for `<ReactMarkdown>` that preserves `buzz://message?…`
 * links. The default transform strips unknown schemes (returns `""`) before
 * the `a` component override can see them, which would break copy → paste →
 * click end-to-end. Everything else delegates to `defaultUrlTransform`.
 */
export function messageLinkUrlTransform(value: string, key: string): string {
  if (key === "href" && isMessageLink(value)) {
    return value;
  }
  return defaultUrlTransform(value);
}

export function getReactNodeText(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") {
    return String(node);
  }

  if (Array.isArray(node)) {
    return node.map(getReactNodeText).join("");
  }

  if (React.isValidElement<{ children?: React.ReactNode }>(node)) {
    return getReactNodeText(node.props.children);
  }

  return "";
}
