import type { ComponentProps } from "react";
import { Checkbox as Primitive } from "@base-ui/react/checkbox";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return (
    <Primitive.Root {...props} className={skin("bui-checkbox", className)} />
  );
}

function StyledIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Indicator>) {
  return (
    <Primitive.Indicator
      {...props}
      className={skin("bui-check-indicator", className)}
    />
  );
}

/** Checkbox behavior primitives with the Buzz visual roles. */
export const Checkbox = { Root: StyledRoot, Indicator: StyledIndicator };
