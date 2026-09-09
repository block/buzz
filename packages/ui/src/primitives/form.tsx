import type { ComponentProps } from "react";
import { Form as Primitive } from "@base-ui/react/form";
import { skin } from "../classes";

function StyledRoot({ className, ...props }: ComponentProps<typeof Primitive>) {
  return <Primitive {...props} className={skin("bui-stack", className)} />;
}

/** Form behavior primitives with the Buzz visual roles. */
export const Form = { Root: StyledRoot };
