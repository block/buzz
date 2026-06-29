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
