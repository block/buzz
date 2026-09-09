import type { ComponentProps } from "react";
import { Tooltip as Primitive } from "@base-ui/react/tooltip";
import { skin } from "../classes";

function StyledPositioner({
  className,
  ...props
}: ComponentProps<typeof Primitive.Positioner>) {
  return (
    <Primitive.Positioner
      {...props}
      className={skin("bui-positioner", className)}
    />
  );
}

function StyledPopup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Popup>) {
  return (
    <Primitive.Popup {...props} className={skin("bui-tooltip", className)} />
  );
}

/** Tooltip behavior primitives with the Buzz visual roles. */
export const Tooltip = {
  Provider: Primitive.Provider,
  Root: Primitive.Root,
  Trigger: Primitive.Trigger,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
};
