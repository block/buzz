import type { ComponentProps } from "react";
import { Toggle as Primitive } from "@base-ui/react/toggle";
import { skin } from "../classes";

function StyledToggle({
  className,
  ...props
}: ComponentProps<typeof Primitive>) {
  return <Primitive {...props} className={skin("bui-toggle", className)} />;
}

/** Toggle behavior primitives with the Buzz visual roles. */
export const Toggle = { Toggle: StyledToggle };
