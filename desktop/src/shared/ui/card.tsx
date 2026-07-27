import * as React from "react";
import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/shared/lib/cn";
import "./card-texture.css";

/**
 * `variant="textured"` renders the baked nine-slice powder texture
 * (`card-texture.css`). The asset bakes the card surface INTO the image:
 * a solid white center that feathers out into speckle, with a transparent
 * powder bleed extending 96px beyond the layout box.
 *
 * Usage contract:
 * - The baked white center IS the card surface. Never layer an opaque
 *   background (`bg-card`, `bg-white`, …) on top of the texture — it
 *   covers the feathered edge and reintroduces a visible hard border.
 * - Default padding is the safe inset (`--buzz-card-textured-safe-inset`),
 *   which keeps content on the fully opaque center. Add more padding as
 *   the content needs; go below it only if the content tolerates sitting
 *   on the semi-transparent fade (e.g. a transparent input).
 * - The card resizes freely — the nine-slice stretches the solid center
 *   with plain CSS. No image regeneration is ever needed for sizing or
 *   padding changes.
 * - Give the layout around the card room for the 96px outer bleed; an
 *   `overflow: hidden` ancestor will clip it.
 * - For modals, use `DialogContent surface="textured"` instead of
 *   composing this by hand — it places the close button inside the
 *   solid center for you.
 */
const cardVariants = cva("text-card-foreground", {
  variants: {
    variant: {
      default: "rounded-xl border border-border/70 bg-card/80 shadow-xs",
      textured:
        // flex + justify-center: the variant enforces a min size (see
        // card-texture.css); when that floor stretches the card beyond its
        // content, the content stays vertically centered instead of pinning
        // to the top padding edge.
        "buzz-card-textured relative isolate flex flex-col justify-center rounded-none border-0 p-[var(--buzz-card-textured-safe-inset)] shadow-none",
    },
  },
  defaultVariants: {
    variant: "default",
  },
});

export interface CardProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof cardVariants> {
  asChild?: boolean;
}

const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ asChild = false, className, variant, ...props }, ref) => {
    const Comp = asChild ? Slot : "div";
    return (
      <Comp
        ref={ref}
        className={cn(cardVariants({ variant, className }))}
        {...props}
      />
    );
  },
);
Card.displayName = "Card";

const CardHeader = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("flex flex-col space-y-1.5 p-6", className)}
    {...props}
  />
));
CardHeader.displayName = "CardHeader";

const CardTitle = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn(
      "text-2xl font-semibold leading-none tracking-tight",
      className,
    )}
    {...props}
  />
));
CardTitle.displayName = "CardTitle";

const CardDescription = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("text-sm text-muted-foreground", className)}
    {...props}
  />
));
CardDescription.displayName = "CardDescription";

const CardContent = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn("p-6 pt-0", className)} {...props} />
));
CardContent.displayName = "CardContent";

const CardFooter = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div
    ref={ref}
    className={cn("flex items-center p-6 pt-0", className)}
    {...props}
  />
));
CardFooter.displayName = "CardFooter";

export {
  Card,
  CardHeader,
  CardFooter,
  CardTitle,
  CardDescription,
  CardContent,
};
