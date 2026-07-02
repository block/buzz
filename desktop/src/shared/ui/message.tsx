import type * as React from "react";
import { Slot } from "@radix-ui/react-slot";

import { cn } from "@/shared/lib/cn";

type MessageProps = React.ComponentProps<"div"> & {
  asChild?: boolean;
  side?: "left" | "right" | "center";
};

function Message({
  asChild,
  className,
  side = "left",
  ...props
}: MessageProps) {
  const Comp = asChild ? Slot : "div";

  return (
    <Comp
      className={cn(
        "group/message flex min-w-0 gap-3 px-5 py-2",
        side === "right" && "justify-end",
        side === "center" && "justify-center",
        className,
      )}
      data-side={side}
      data-slot="message"
      {...props}
    />
  );
}

function MessageAvatar({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("mt-0.5 shrink-0", className)}
      data-slot="message-avatar"
      {...props}
    />
  );
}

function MessageContent({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("min-w-0 max-w-[min(42rem,78%)]", className)}
      data-slot="message-content"
      {...props}
    />
  );
}

function MessageHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn(
        "mb-1 flex min-w-0 items-baseline gap-2 text-xs text-muted-foreground",
        className,
      )}
      data-slot="message-header"
      {...props}
    />
  );
}

export { Message, MessageAvatar, MessageContent, MessageHeader };
