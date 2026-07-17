import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/shared/lib/cn";

const cardVariants = cva("text-card-foreground", {
  variants: {
    variant: {
      default: "rounded-xl border border-border/70 bg-card/80 shadow-xs",
      textured: "relative isolate rounded-none border-0 shadow-none",
    },
  },
  defaultVariants: {
    variant: "default",
  },
});

export interface CardProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof cardVariants> {}

/**
 * Figma's "Texture" effect (Size 0.94, Radius ~93, unclipped), ported to a
 * scalable SVG filter.
 *
 * Figma's render is a noise-dithered blur, not a displacement of the hard
 * shape (dev-mode's `feDisplacementMap` export keeps the crisp rectangle
 * visible — wrong look). The real recipe:
 *
 * 1. Blur the shape's alpha — a smooth ramp centered on the card edge:
 *    solid a fade-width inside the bounds, dissolving to nothing a
 *    fade-width outside.
 * 2. Per-pixel fractal noise acts as a threshold: a pixel becomes a solid
 *    white speck only where ramp alpha exceeds the noise value
 *    (`alpha − noise`, then a steep slope to snap specks to full white).
 *
 * Dot density then follows the smooth gradient, so the shape's edge fully
 * dissolves into powder with no visible rectangle.
 */
/** Fade width — blurred alpha ramp; ~Figma's texture radius / 3. */
const TEXTURE_BLUR_PX = 66;
/**
 * Corner squaring. A Gaussian blur rounds corners: a corner pixel sees the
 * shape on ~¼ of its neighborhood (vs ½ along an edge), so corner alpha
 * collapses into a pill contour. Dilating first (square kernel) pushes the
 * corners out square, boosting corner alpha before the blur. Half the blur
 * width is a good default; raise toward the full blur for squarer corners.
 */
const TEXTURE_DILATE_PX = Math.round(TEXTURE_BLUR_PX * 0.85);
/**
 * Threshold bias compensating for the dilation. Dilating shifts the whole
 * fade outward — edge alpha lands at Φ(dilate/blur) instead of 0.5.
 * Subtracting the difference in the dither re-centers the 50%-density line
 * on the card's true edge. (Recompute as Φ(dilate/blur) − 0.5 if the
 * dilate:blur ratio changes; for ratio 0.85 it's ≈ 0.302.)
 */
const TEXTURE_THRESHOLD_BIAS = 0.302;
/** SVG bleed: dilate + blur tail (~2.5σ) so no speck is clipped. */
const TEXTURE_BLEED_PX = 224;

function TexturedCardDecoration() {
  const filterId = `card-texture-${React.useId().replace(/:/g, "")}`;

  return (
    <>
      <svg
        aria-hidden="true"
        className="pointer-events-none absolute overflow-visible"
        focusable="false"
        style={{
          inset: -TEXTURE_BLEED_PX,
          height: `calc(100% + ${TEXTURE_BLEED_PX * 2}px)`,
          width: `calc(100% + ${TEXTURE_BLEED_PX * 2}px)`,
        }}
      >
        <defs>
          {/* The SVG wrapper already includes the full configured bleed, so
              the filter only needs to cover that wrapper. Keeping the filter
              region at its default bounds avoids rasterizing another large,
              invisible percentage-based surface around it. */}
          <filter
            id={filterId}
            x="0"
            y="0"
            width="100%"
            height="100%"
            colorInterpolationFilters="sRGB"
            filterUnits="userSpaceOnUse"
          >
            <feMorphology
              in="SourceAlpha"
              operator="dilate"
              radius={TEXTURE_DILATE_PX}
              result="squared"
            />
            <feGaussianBlur
              in="squared"
              result="ramp"
              stdDeviation={TEXTURE_BLUR_PX}
            />
            <feTurbulence
              baseFrequency="0.999"
              numOctaves="3"
              result="grain"
              seed="5315"
              type="fractalNoise"
            />
            {/* Move the noise's red channel into alpha so it can threshold
                the ramp. */}
            <feColorMatrix
              in="grain"
              result="grainAlpha"
              type="matrix"
              values="0 0 0 0 0
                      0 0 0 0 0
                      0 0 0 0 0
                      1 0 0 0 0"
            />
            {/* Dither: ramp alpha minus noise, minus the dilation bias (k4)
                so the 50%-density line sits back on the card's true edge.
                Positive only where the ramp beats the noise, so speck
                density follows the gradient. */}
            <feComposite
              in="ramp"
              in2="grainAlpha"
              k1="0"
              k2="1"
              k3="-1"
              k4={-TEXTURE_THRESHOLD_BIAS}
              operator="arithmetic"
              result="dithered"
            />
            {/* Snap surviving specks toward full opacity — Figma's dots are
                binary white, not translucent. */}
            <feComponentTransfer in="dithered" result="specks">
              <feFuncA intercept="0" slope="8" type="linear" />
            </feComponentTransfer>
            <feFlood floodColor="white" result="white" />
            <feComposite in="white" in2="specks" operator="in" />
          </filter>
        </defs>
        <rect
          fill="white"
          filter={`url(#${filterId})`}
          height={`calc(100% - ${TEXTURE_BLEED_PX * 2}px)`}
          width={`calc(100% - ${TEXTURE_BLEED_PX * 2}px)`}
          x={TEXTURE_BLEED_PX}
          y={TEXTURE_BLEED_PX}
        />
      </svg>
      {/* Solid core, feathered. A CSS blur turns the hard-edged white span
          into a smooth alpha ramp, so it dissolves into the speck layer
          instead of meeting it with a visible contour. Inset (blur/2) and
          feather (blur/3) are chosen so opposite fades never overlap in the
          center — content stays on opaque white. */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute rounded-[inherit] bg-white"
        data-card-surface="true"
        style={{
          filter: `blur(${TEXTURE_BLUR_PX / 3}px)`,
          inset: TEXTURE_BLUR_PX / 2,
        }}
      />
    </>
  );
}

const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ children, className, variant, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(cardVariants({ variant, className }))}
      {...props}
    >
      {variant === "textured" ? <TexturedCardDecoration /> : null}
      {variant === "textured" ? (
        <div className="relative z-10">{children}</div>
      ) : (
        children
      )}
    </div>
  ),
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
    className={cn("font-semibold leading-none tracking-tight", className)}
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
