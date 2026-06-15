import * as React from "react";

import { cn } from "@/shared/lib/cn";

import { DialogContent, DialogHeader, DialogTitle } from "./dialog";

type ChooserDialogContentProps = React.ComponentPropsWithoutRef<
  typeof DialogContent
> & {
  description?: React.ReactNode;
  footer?: React.ReactNode;
  footerClassName?: string;
  footerTestId?: string;
  contentClassName?: string;
  headerClassName?: string;
  headerTestId?: string;
  scrollAreaClassName?: string;
  scrollAreaTestId?: string;
  title: React.ReactNode;
};

export const ChooserDialogContent = React.forwardRef<
  React.ElementRef<typeof DialogContent>,
  ChooserDialogContentProps
>(
  (
    {
      children,
      className,
      contentClassName,
      description: _description,
      footer,
      footerClassName,
      footerTestId,
      headerClassName,
      headerTestId,
      scrollAreaClassName,
      scrollAreaTestId,
      title,
      "aria-describedby": ariaDescribedBy,
      ...props
    },
    ref,
  ) => (
    <DialogContent
      aria-describedby={ariaDescribedBy}
      className={cn(
        "flex max-h-[85vh] flex-col gap-0 overflow-hidden p-0",
        className,
      )}
      ref={ref}
      {...props}
    >
      <DialogHeader
        className={cn("shrink-0 px-6 py-5 pr-14", headerClassName)}
        data-testid={headerTestId}
      >
        <DialogTitle>{title}</DialogTitle>
      </DialogHeader>

      <div
        className={cn(
          "min-h-0 flex-1 overflow-y-auto px-6",
          scrollAreaClassName,
        )}
        data-testid={scrollAreaTestId}
      >
        <div className={cn("py-5", contentClassName)}>{children}</div>
      </div>

      {footer ? (
        <div
          className={cn(
            "flex shrink-0 border-t border-border/60 px-6 py-4",
            footerClassName,
          )}
          data-testid={footerTestId}
        >
          {footer}
        </div>
      ) : null}
    </DialogContent>
  ),
);
ChooserDialogContent.displayName = "ChooserDialogContent";
