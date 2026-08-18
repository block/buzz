import type { CSSProperties } from "react";

export const POPOVER_SURFACE_CLASS =
  "border border-border/60 bg-[color-mix(in_srgb,hsl(var(--background))_80%,hsl(var(--muted))_20%)] text-popover-foreground";

export const POPOVER_RADIX_MOTION_CLASS =
  "duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] data-[state=closed]:duration-100 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0 data-[state=closed]:zoom-out-95 data-[state=open]:zoom-in-95 motion-reduce:animate-none";

export const POPOVER_RADIX_SIDE_MOTION_CLASS =
  "data-[side=bottom]:slide-in-from-top-1 data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1 data-[side=top]:slide-in-from-bottom-1";

export const POPOVER_CUSTOM_ENTER_MOTION_CLASS =
  "animate-in fade-in-0 zoom-in-95 duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:animate-none";

export const POPOVER_SHADOW =
  "0 6px 18px lch(0% 0 0 / 0.02), 0 3px 9px lch(0% 0 0 / 0.04), 0 1px 1px lch(0% 0 0 / 0.04)";

export const POPOVER_SHADOW_STYLE: CSSProperties = {
  boxShadow: POPOVER_SHADOW,
};
