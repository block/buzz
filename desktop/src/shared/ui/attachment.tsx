import * as React from "react";
import { Slot } from "@radix-ui/react-slot";

import { cn } from "@/shared/lib/cn";
import { Button, type ButtonProps } from "@/shared/ui/button";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";

type AttachmentProps = React.ComponentProps<"div"> & {
  orientation?: "horizontal" | "vertical";
  size?: "default" | "sm" | "xs";
  state?: "idle" | "uploading" | "processing" | "error" | "done";
};

function Attachment({
  className,
  orientation = "horizontal",
  size = "default",
  state = "done",
  ...props
}: AttachmentProps) {
  const attachmentRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(attachmentRef);

  return (
    <div
      ref={attachmentRef}
      className={cn(
        "group/attachment relative flex min-w-0 gap-3 overflow-hidden rounded-2xl border border-border/70 bg-muted/30 text-left transition-colors",
        "hover:border-border hover:bg-muted/50 data-[state=error]:border-destructive/40 data-[state=error]:bg-destructive/10",
        "focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
        orientation === "horizontal" && "items-center",
        orientation === "vertical" && "flex-col",
        size === "default" && "px-3 py-2.5",
        size === "sm" && "px-2.5 py-2",
        size === "xs" && "gap-2 px-2 py-1.5",
        className,
      )}
      data-orientation={orientation}
      data-slot="attachment"
      data-state={state}
      {...props}
    />
  );
}

type AttachmentMediaProps = React.ComponentProps<"div"> & {
  variant?: "icon" | "image";
};

function AttachmentMedia({
  className,
  variant = "icon",
  ...props
}: AttachmentMediaProps) {
  return (
    <div
      className={cn(
        "flex shrink-0 items-center justify-center overflow-hidden rounded-md bg-background text-muted-foreground",
        variant === "icon" && "h-9 w-9",
        variant === "image" && "aspect-square h-12 w-12",
        "[&_svg]:h-4 [&_svg]:w-4",
        className,
      )}
      data-slot="attachment-media"
      data-variant={variant}
      {...props}
    />
  );
}

function AttachmentContent({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("min-w-0 flex-1", className)}
      data-slot="attachment-content"
      {...props}
    />
  );
}

function AttachmentTitle({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "truncate text-sm font-semibold leading-5 text-foreground group-data-[state=processing]/attachment:animate-pulse group-data-[state=uploading]/attachment:animate-pulse",
        className,
      )}
      data-slot="attachment-title"
      {...props}
    />
  );
}

function AttachmentDescription({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "truncate text-xs leading-4 text-muted-foreground",
        className,
      )}
      data-slot="attachment-description"
      {...props}
    />
  );
}

function AttachmentActions({
  className,
  ...props
}: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "relative z-20 flex shrink-0 items-center gap-1",
        className,
      )}
      data-slot="attachment-actions"
      {...props}
    />
  );
}

type AttachmentActionProps = ButtonProps & {
  asChild?: boolean;
};

function AttachmentAction({
  className,
  size = "icon-xs",
  variant = "ghost",
  ...props
}: AttachmentActionProps) {
  return (
    <Button
      className={cn(
        "relative z-20 text-muted-foreground hover:bg-muted hover:text-foreground",
        className,
      )}
      data-slot="attachment-action"
      size={size}
      variant={variant}
      {...props}
    />
  );
}

type AttachmentTriggerProps = React.ComponentProps<"button"> & {
  asChild?: boolean;
};

function AttachmentTrigger({
  asChild,
  className,
  type = "button",
  ...props
}: AttachmentTriggerProps) {
  const Comp = asChild ? Slot : "button";

  return (
    <Comp
      className={cn(
        "absolute inset-0 z-10 rounded-2xl focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
        className,
      )}
      data-slot="attachment-trigger"
      type={asChild ? undefined : type}
      {...props}
    />
  );
}

function AttachmentGroup({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "flex min-w-0 gap-2 overflow-x-auto overscroll-x-contain pb-1",
        className,
      )}
      data-slot="attachment-group"
      {...props}
    />
  );
}

export {
  Attachment,
  AttachmentAction,
  AttachmentActions,
  AttachmentContent,
  AttachmentDescription,
  AttachmentGroup,
  AttachmentMedia,
  AttachmentTitle,
  AttachmentTrigger,
};
