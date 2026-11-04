import * as React from "react";
import { SendHorizontal } from "lucide-react";

import {
  ATTACHMENT_DRAG_MIME,
  attachmentDragPreview,
  parseAttachmentDragPayload,
} from "@/features/attachment-drag/lib/attachmentDragPayload";
import type { AttachmentDragPayload } from "@/features/attachment-drag/lib/attachmentDragPayload";
import { cn } from "@/shared/lib/cn";

function hasAttachmentDragPayload(event: React.DragEvent): boolean {
  return Array.from(event.dataTransfer.types).includes(ATTACHMENT_DRAG_MIME);
}

export function AttachmentChannelDropTarget({
  channelName,
  children,
  className,
  disabled = false,
  onDropAttachment,
}: {
  channelName: string;
  children: React.ReactNode;
  className?: string;
  disabled?: boolean;
  onDropAttachment?: (payload: AttachmentDragPayload) => Promise<void> | void;
}) {
  const [isOver, setIsOver] = React.useState(false);
  const [preview, setPreview] = React.useState<string | null>(null);

  const clear = React.useCallback(() => {
    setIsOver(false);
    setPreview(null);
  }, []);

  if (!onDropAttachment || disabled) {
    return <>{children}</>;
  }

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: Native drag/drop needs handlers on the wrapper around the existing sidebar button.
    <div
      className={cn("relative", className)}
      onDragEnter={(event) => {
        if (!hasAttachmentDragPayload(event)) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = "copy";
        setIsOver(true);
        const payload = parseAttachmentDragPayload(
          event.dataTransfer.getData(ATTACHMENT_DRAG_MIME),
        );
        setPreview(
          payload ? attachmentDragPreview(payload).label : "attachment",
        );
      }}
      onDragOver={(event) => {
        if (!hasAttachmentDragPayload(event)) return;
        event.preventDefault();
        event.stopPropagation();
        event.dataTransfer.dropEffect = "copy";
        setIsOver(true);
      }}
      onDragLeave={(event) => {
        const nextTarget = event.relatedTarget;
        if (
          nextTarget instanceof Node &&
          event.currentTarget.contains(nextTarget)
        ) {
          return;
        }
        clear();
      }}
      onDrop={(event) => {
        if (!hasAttachmentDragPayload(event)) return;
        event.preventDefault();
        event.stopPropagation();
        const payload = parseAttachmentDragPayload(
          event.dataTransfer.getData(ATTACHMENT_DRAG_MIME),
        );
        clear();
        if (payload) void onDropAttachment(payload);
      }}
    >
      {children}
      {isOver ? (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center rounded-md bg-sidebar-accent/90 text-sidebar-foreground shadow-sm ring-2 ring-primary/45 backdrop-blur-sm"
          data-testid={`attachment-drop-target-${channelName}`}
        >
          <span className="flex max-w-[calc(100%-0.5rem)] items-center gap-1.5 truncate rounded-full bg-sidebar px-2 py-1 text-2xs font-medium shadow-sm ring-1 ring-sidebar-border">
            <SendHorizontal className="h-3 w-3 shrink-0 text-primary" />
            <span className="truncate">
              Send {preview ?? "attachment"} here
            </span>
          </span>
        </div>
      ) : null}
    </div>
  );
}
