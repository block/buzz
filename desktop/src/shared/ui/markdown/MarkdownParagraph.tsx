import * as React from "react";
import { isAudioAttachment } from "@/features/messages/lib/audioAttachment";
import {
  classifyChildren,
  hasBlockMedia,
  isImageOnlyParagraph,
} from "../markdownUtils";
import { BubbleMediaParagraph } from "./BubbleMediaParagraph";
import { ImageMosaic } from "./ImageMosaic";
import { useMarkdownRuntime } from "./runtimeContext";

/** Preserve block media, image mosaics, and inline audio paragraph semantics. */
export function MarkdownParagraph({
  children,
}: {
  children?: React.ReactNode;
}) {
  const { imetaByUrl } = useMarkdownRuntime();
  // Detect media-only paragraphs (images + <br> from remarkBreaks).
  // Multi-image: render as a compact, count-aware mosaic. Two images split
  // a row, three form a hero-and-stack triptych, and larger odd counts let
  // the final image span both columns.
  // Single media: render as a plain <div> to avoid invalid <p><div> nesting
  // (the img component returns block-level wrappers for lightbox/video).
  const childArray = React.Children.toArray(children);
  const { imageChildren } = classifyChildren(childArray);
  const hasAudioAttachment = childArray.some(
    (child) =>
      React.isValidElement<{ href?: string }>(child) &&
      typeof child.props.href === "string" &&
      isAudioAttachment(imetaByUrl?.get(child.props.href)),
  );

  if (isImageOnlyParagraph(childArray)) {
    return <ImageMosaic>{imageChildren}</ImageMosaic>;
  }

  if (hasBlockMedia(childArray)) {
    return <BubbleMediaParagraph>{children}</BubbleMediaParagraph>;
  }
  if (hasAudioAttachment) return <div>{children}</div>;

  return <p>{children}</p>;
}
