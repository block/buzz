import type { ComponentProps } from "react";
import { Menubar as Primitive } from "@base-ui/react/menubar";
import { skin } from "../classes";

function StyledRoot({ className, ...props }: ComponentProps<typeof Primitive>) {
  return <Primitive {...props} className={skin("bui-toolbar", className)} />;
}

/** Menubar behavior primitives with the Buzz visual roles. */
export const Menubar = { Root: StyledRoot };
