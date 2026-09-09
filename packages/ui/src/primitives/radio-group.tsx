import type { ComponentProps } from "react";
import { RadioGroup as Primitive } from "@base-ui/react/radio-group";
import { skin } from "../classes";

function StyledRoot({ className, ...props }: ComponentProps<typeof Primitive>) {
  return <Primitive {...props} className={skin("bui-stack", className)} />;
}

/** RadioGroup behavior primitives with the Buzz visual roles. */
export const RadioGroup = { Root: StyledRoot };
