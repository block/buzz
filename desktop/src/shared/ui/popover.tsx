import * as React from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";

import { cn } from "@/shared/lib/cn";
import {
  type CardTextureSize,
  type CardTextureTone,
  texturedSurfaceClasses,
} from "@/shared/ui/card";
import {
  POPOVER_RADIX_MOTION_CLASS,
  POPOVER_RADIX_SIDE_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";

// Radix Popover has no hover timing API: controlled hover popovers must use this
// shared dwell default themselves. Keep click and keyboard opens immediate.
export const DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS = 500;

const Popover = PopoverPrimitive.Root;

const PopoverTrigger = PopoverPrimitive.Trigger;

const PopoverAnchor = PopoverPrimitive.Anchor;

type PopoverContentProps = React.ComponentPropsWithoutRef<
  typeof PopoverPrimitive.Content
> & {
  portalled?: boolean;
  surface?: "default" | "textured";
  textureSize?: CardTextureSize;
  textureTone?: CardTextureTone;
};

const PopoverContent = React.forwardRef<
  React.ElementRef<typeof PopoverPrimitive.Content>,
  PopoverContentProps
>(
  (
    {
      className,
      align = "center",
      portalled = true,
      sideOffset,
      style,
      surface = "default",
      textureSize = "regular",
      textureTone = "light",
      ...props
    },
    ref,
  ) => {
    const content = (
      <PopoverPrimitive.Content
        ref={ref}
        align={align}
        sideOffset={sideOffset ?? (surface === "textured" ? 24 : 4)}
        className={cn(
          "z-50 w-72 origin-(--radix-popover-content-transform-origin) outline-hidden",
          POPOVER_RADIX_MOTION_CLASS,
          POPOVER_RADIX_SIDE_MOTION_CLASS,
          surface === "default" && cn("rounded-xl p-4", POPOVER_SURFACE_CLASS),
          surface === "textured" &&
            texturedSurfaceClasses({
              size: textureSize,
              tone: textureTone,
            }),
          className,
        )}
        style={{
          ...(surface === "default" ? POPOVER_SHADOW_STYLE : {}),
          ...style,
        }}
        {...props}
      />
    );

    return portalled ? (
      <PopoverPrimitive.Portal>{content}</PopoverPrimitive.Portal>
    ) : (
      content
    );
  },
);
PopoverContent.displayName = PopoverPrimitive.Content.displayName;

export { Popover, PopoverTrigger, PopoverContent, PopoverAnchor };
