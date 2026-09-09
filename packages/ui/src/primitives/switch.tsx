import type { ComponentProps } from "react";
import { Switch as Primitive } from "@base-ui/react/switch";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return (
    <Primitive.Root {...props} className={skin("bui-switch", className)} />
  );
}

function StyledThumb({
  className,
  ...props
}: ComponentProps<typeof Primitive.Thumb>) {
  return (
    <Primitive.Thumb
      {...props}
      className={skin("bui-switch-thumb", className)}
    />
  );
}

/** Switch behavior primitives with the Buzz visual roles. */
export const Switch = { Root: StyledRoot, Thumb: StyledThumb };
