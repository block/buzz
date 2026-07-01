import * as React from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";

import { cn } from "@/shared/lib/cn";
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
  portalled?: boolean;
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
      sideOffset = 4,
      style,
      ...props
    },
    ref,
  ) => {
    const content = (
      <PopoverPrimitive.Content
        ref={ref}
        align={align}
        sideOffset={sideOffset}
        className={cn(
          "z-50 w-72 origin-(--radix-popover-content-transform-origin) rounded-xl p-4 outline-hidden",
          POPOVER_RADIX_MOTION_CLASS,
          POPOVER_RADIX_SIDE_MOTION_CLASS,
          POPOVER_SURFACE_CLASS,
          className,
        )}
        style={{ ...POPOVER_SHADOW_STYLE, ...style }}
        {...props}
      />
    );

    if (!portalled) {
      return content;
    }

    return <PopoverPrimitive.Portal>{content}</PopoverPrimitive.Portal>;
  },
);
PopoverContent.displayName = PopoverPrimitive.Content.displayName;

export { Popover, PopoverTrigger, PopoverContent, PopoverAnchor };
