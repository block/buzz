import type { ComponentProps } from "react";
import { Radio as Primitive } from "@base-ui/react/radio";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-radio", className)} />;
}

function StyledIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Indicator>) {
  return (
    <Primitive.Indicator
      {...props}
      className={skin("bui-radio-indicator", className)}
    />
  );
}

/** Radio behavior primitives with the Buzz visual roles. */
export const Radio = { Root: StyledRoot, Indicator: StyledIndicator };
