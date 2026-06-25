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

  const props = child.props as {
    children?: React.ReactNode;
    node?: { tagName?: unknown };
    [key: string]: unknown;
  };
  const node = props?.node as { tagName?: unknown } | undefined;

  if (props?.["data-block-media"] != null || node?.tagName === "img") {
    return true;
  }

  return React.Children.toArray(props.children).some(isBlockMedia);
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
