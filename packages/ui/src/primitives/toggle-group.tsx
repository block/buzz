import type { ComponentProps } from "react";
import { ToggleGroup as Primitive } from "@base-ui/react/toggle-group";
import { skin } from "../classes";

function StyledToggleGroup({
  className,
  ...props
}: ComponentProps<typeof Primitive>) {
  return (
    <Primitive {...props} className={skin("bui-toggle-group", className)} />
  );
}

/** ToggleGroup behavior primitives with the Buzz visual roles. */
export const ToggleGroup = { ToggleGroup: StyledToggleGroup };
