import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button, type ButtonProps } from "@/shared/ui/button";

/**
 * Canonical text action used throughout Settings: a compact, pill-shaped
 * button. Callers may still choose a semantic variant (primary, destructive,
 * outline), while shape and size stay consistent.
 */
export const SettingsActionButton = React.forwardRef<
  HTMLButtonElement,
  ButtonProps
>(({ className, size = "sm", ...props }, ref) => (
  <Button
    className={cn("rounded-full !shadow-none", className)}
    ref={ref}
    size={size}
    {...props}
  />
));

SettingsActionButton.displayName = "SettingsActionButton";
