"use client";

import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import "./card-texture.css";
import { MODAL_BACKDROP_BLUR_CLASS } from "@/shared/ui/modalBackdrop";
import {
  MODAL_CONTENT_MOTION_CLASS,
  MODAL_OVERLAY_MOTION_CLASS,
} from "@/shared/ui/modalMotion";

const Dialog = DialogPrimitive.Root;
const DialogTrigger = DialogPrimitive.Trigger;
const DialogPortal = DialogPrimitive.Portal;
const DialogClose = DialogPrimitive.Close;

const DialogOverlay = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Overlay>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Overlay>
>(({ className, ...props }, ref) => {
  const { isDark } = useTheme();

  return (
    <DialogPrimitive.Overlay
      className={cn(
        "fixed inset-0 z-50",
        MODAL_OVERLAY_MOTION_CLASS,
        MODAL_BACKDROP_BLUR_CLASS,
        isDark ? "bg-black/60" : "bg-black/10",
        className,
      )}
      ref={ref}
      {...props}
    />
  );
});
DialogOverlay.displayName = DialogPrimitive.Overlay.displayName;

type DialogContentProps = React.ComponentPropsWithoutRef<
  typeof DialogPrimitive.Content
> & {
  /** Extra classes for the built-in close button (e.g. a themed icon color). */
  closeButtonClassName?: string;
  overlayVariant?: "default" | "transparent";
  showCloseButton?: boolean;
  /**
   * - `default`: standard opaque dialog panel (rounded, shadowed).
   * - `none`: no surface — the caller composes its own.
   * - `textured`: the baked nine-slice powder card (`Card variant="textured"`)
   *   IS the dialog surface. Content and the close button are automatically
   *   kept on the solid center of the texture via its safe inset.
   */
  surface?: "default" | "none" | "textured";
};

const DialogContent = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Content>,
  DialogContentProps
>(
  (
    {
      className,
      children,
      closeButtonClassName,
      overlayVariant = "default",
      showCloseButton = true,
      surface = "default",
      ...props
    },
    ref,
  ) => (
    <DialogPortal>
      <DialogOverlay
        data-testid="dialog-overlay"
        className={
          overlayVariant === "transparent"
            ? "bg-transparent backdrop-blur-none"
            : undefined
        }
      />
      <div
        className={cn(
          "pointer-events-none fixed inset-0 z-50 grid place-items-center overflow-x-hidden overflow-y-auto",
          // The textured surface bleeds a 96px powder band beyond its layout
          // box (see card-texture.css). Give the wrapper enough padding that
          // the bleed isn't clipped by this scroll container; every other
          // surface keeps the standard gutter.
          surface === "textured"
            ? "p-[calc(6rem+1rem)] max-sm:p-[calc(6rem-1.5rem)]"
            : "p-4",
        )}
      >
        <DialogPrimitive.Content
          className={cn(
            "pointer-events-auto relative grid w-[calc(100vw-2rem)] max-w-2xl gap-4 outline-hidden",
            surface === "default" && "rounded-2xl bg-background p-6 shadow-2xl",
            surface === "none" && "bg-transparent p-0 shadow-none",
            surface === "textured" &&
              "buzz-card-textured isolate box-border w-full rounded-none border-0 bg-transparent p-[var(--buzz-card-textured-safe-inset)] shadow-none",
            MODAL_CONTENT_MOTION_CLASS,
            className,
          )}
          ref={ref}
          {...props}
        >
          {children}
          {showCloseButton ? (
            <DialogPrimitive.Close
              className={cn(
                "absolute flex h-8 w-8 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus-visible:ring-1 focus-visible:ring-ring",
                // On the textured surface the layout edge sits in the powder
                // fade; dock the close button at the safe-inset corner so it
                // stays on the solid center of the texture.
                surface === "textured"
                  ? "right-[var(--buzz-card-textured-safe-inset)] top-[var(--buzz-card-textured-safe-inset)] -mr-2 -mt-2"
                  : "right-4 top-4",
                closeButtonClassName,
              )}
            >
              <X className="h-4 w-4" />
              <span className="sr-only">Close</span>
            </DialogPrimitive.Close>
          ) : null}
        </DialogPrimitive.Content>
      </div>
    </DialogPortal>
  ),
);
DialogContent.displayName = DialogPrimitive.Content.displayName;

const DialogHeader = ({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn("flex flex-col space-y-2 text-left", className)}
    {...props}
  />
);
DialogHeader.displayName = "DialogHeader";

const DialogFooter = ({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) => (
  <div
    className={cn(
      "flex flex-col-reverse gap-2 sm:flex-row sm:justify-end",
      className,
    )}
    {...props}
  />
);
DialogFooter.displayName = "DialogFooter";

const DialogTitle = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Title>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Title>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Title
    className={cn("text-xl font-semibold tracking-tight", className)}
    ref={ref}
    {...props}
  />
));
DialogTitle.displayName = DialogPrimitive.Title.displayName;

const DialogDescription = React.forwardRef<
  React.ElementRef<typeof DialogPrimitive.Description>,
  React.ComponentPropsWithoutRef<typeof DialogPrimitive.Description>
>(({ className, ...props }, ref) => (
  <DialogPrimitive.Description
    className={cn("text-sm text-muted-foreground", className)}
    ref={ref}
    {...props}
  />
));
DialogDescription.displayName = DialogPrimitive.Description.displayName;

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
