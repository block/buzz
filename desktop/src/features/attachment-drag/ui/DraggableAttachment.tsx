import * as React from "react";

import {
  ATTACHMENT_DRAG_MIME,
  type AttachmentDragItem,
  type AttachmentDragPayload,
  attachmentDragPreview,
  serializeAttachmentDragPayload,
} from "@/features/attachment-drag/lib/attachmentDragPayload";
import { cn } from "@/shared/lib/cn";

export function DraggableAttachment({
  children,
  className,
  item,
  sourceAuthor,
  sourceChannelId,
  sourceMessageId,
  sourceThreadRootId,
}: {
  children: React.ReactNode;
  className?: string;
  item: AttachmentDragItem;
  sourceAuthor?: string | null;
  sourceChannelId?: string | null;
  sourceMessageId: string;
  sourceThreadRootId?: string | null;
}) {
  const preview = React.useMemo(
    () => attachmentDragPreview({ items: [item] }),
    [item],
  );

  const handleDragStart = React.useCallback(
    (event: React.DragEvent<HTMLSpanElement>) => {
      const payload: AttachmentDragPayload = {
        type: "buzz-attachment-drag",
        version: 1,
        sourceAuthor,
        sourceChannelId,
        sourceMessageId,
        sourceThreadRootId,
        items: [item],
      };
      event.dataTransfer.effectAllowed = "copy";
      event.dataTransfer.setData(
        ATTACHMENT_DRAG_MIME,
        serializeAttachmentDragPayload(payload),
      );
      event.dataTransfer.setData("text/plain", preview.label);
    },
    [
      item,
      preview.label,
      sourceAuthor,
      sourceChannelId,
      sourceMessageId,
      sourceThreadRootId,
    ],
  );

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: Native attachment dragging is pointer-only; keyboard sending stays covered by existing message actions.
    <span
      className={cn(
        "inline-block cursor-grab rounded-2xl active:cursor-grabbing [&_*]:cursor-inherit",
        className,
      )}
      draggable
      onDragStart={handleDragStart}
      title={`Drag ${preview.label} to a channel or DM to send it there`}
    >
      {children}
    </span>
  );
}
