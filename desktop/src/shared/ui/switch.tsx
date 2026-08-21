import * as React from "react";
import * as SwitchPrimitives from "@radix-ui/react-switch";

import { cn } from "@/shared/lib/cn";

const Switch = React.forwardRef<
  React.ComponentRef<typeof SwitchPrimitives.Root>,
  React.ComponentPropsWithoutRef<typeof SwitchPrimitives.Root>
>(
  (
    { className, onPointerDown, onPointerEnter, onPointerLeave, ...props },
    ref,
  ) => {
    const [suppressHoverStretch, setSuppressHoverStretch] =
      React.useState(false);

    return (
      <SwitchPrimitives.Root
        className={cn(
          "settings-switch peer inline-flex h-5 w-9 shrink-0 cursor-pointer items-center rounded-full border-2 border-transparent shadow-none transition-colors duration-[var(--motion-duration-instant)] ease-[var(--motion-ease-standard)] focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-50 data-[state=checked]:bg-primary data-[state=unchecked]:bg-input motion-reduce:transition-none",
          className,
        )}
        data-hover-stretch-suppressed={suppressHoverStretch || undefined}
        onPointerDown={(event) => {
          setSuppressHoverStretch(true);
          onPointerDown?.(event);
        }}
        onPointerEnter={(event) => {
          setSuppressHoverStretch(false);
          onPointerEnter?.(event);
        }}
        onPointerLeave={(event) => {
          setSuppressHoverStretch(false);
          onPointerLeave?.(event);
        }}
        {...props}
        ref={ref}
      >
        <SwitchPrimitives.Thumb
          className="settings-switch-thumb pointer-events-none block h-4 w-6 rounded-full bg-white shadow-none ring-0"
          data-slot="switch-thumb"
        />
      </SwitchPrimitives.Root>
    );
  },
);
Switch.displayName = SwitchPrimitives.Root.displayName;

export { Switch };
