import type { ComponentProps } from "react";
import { CheckboxGroup as Primitive } from "@base-ui/react/checkbox-group";
import { skin } from "../classes";

function StyledRoot({ className, ...props }: ComponentProps<typeof Primitive>) {
  return <Primitive {...props} className={skin("bui-stack", className)} />;
}

/** CheckboxGroup behavior primitives with the Buzz visual roles. */
export const CheckboxGroup = { Root: StyledRoot };
