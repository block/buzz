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

const Popover = PopoverPrimitive.Root;

const PopoverTrigger = PopoverPrimitive.Trigger;

const PopoverAnchor = PopoverPrimitive.Anchor;

type PopoverContentProps = React.ComponentPropsWithoutRef<
  typeof PopoverPrimitive.Content
> & {
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
      sideOffset,
      style,
      surface = "default",
      textureSize = "regular",
      textureTone = "light",
      ...props
    },
    ref,
  ) => (
    <PopoverPrimitive.Portal>
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
    </PopoverPrimitive.Portal>
  ),
);
PopoverContent.displayName = PopoverPrimitive.Content.displayName;

export { Popover, PopoverTrigger, PopoverContent, PopoverAnchor };
