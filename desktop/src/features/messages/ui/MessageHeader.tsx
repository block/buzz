import type * as React from "react";

import { cn } from "@/shared/lib/cn";

type MessageHeaderRowProps = {
  children: React.ReactNode;
  className?: string;
};

export function MessageHeaderRow({
  children,
  className,
}: MessageHeaderRowProps) {
  return (
    <div
      className={cn(
        "flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-0 leading-4",
        className,
      )}
    >
      {children}
    </div>
  );
}

type MessageAuthorTextProps = {
  as?: "div" | "h3" | "span";
  children: React.ReactNode;
  className?: string;
  hoverUnderline?: boolean;
};

export function MessageAuthorText({
  as: Component = "span",
  children,
  className,
  hoverUnderline = false,
}: MessageAuthorTextProps) {
  return (
    <Component
      className={cn(
        "truncate text-sm font-semibold leading-4 tracking-tight",
        hoverUnderline && "hover:underline",
        className,
      )}
    >
      {children}
    </Component>
  );
}
