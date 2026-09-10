import * as React from "react";
import { MessageBubbleContext } from "@/features/messages/ui/MessageBubbleContext";
import { classifyChildren } from "../markdownUtils";

/** Preserve mixed caption/media order while giving bubble captions their own surface. */
export function BubbleMediaParagraph({
  children,
}: {
  children: React.ReactNode;
}) {
  const bubbleViewer = React.useContext(MessageBubbleContext);
  if (bubbleViewer === null) return <div>{children}</div>;
  const parts: React.ReactNode[] = [];
  let caption: React.ReactNode[] = [];
  const flush = () => {
    if (classifyChildren(caption).nonImageChildren.length) {
      parts.push(
        <div key={`caption-${parts.length}`} data-bubble-caption="">
          {caption}
        </div>,
      );
    }
    caption = [];
  };
  for (const child of React.Children.toArray(children)) {
    if (classifyChildren([child]).imageChildren.length) {
      flush();
      parts.push(child);
    } else caption.push(child);
  }
  flush();
  return <div data-bubble-media-paragraph="">{parts}</div>;
}
