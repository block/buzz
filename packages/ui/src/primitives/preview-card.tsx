import type { ComponentProps } from "react";
import { PreviewCard as Primitive } from "@base-ui/react/preview-card";
import { skin } from "../classes";

function StyledTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.Trigger>) {
  return (
    <Primitive.Trigger
      {...props}
      className={skin("bui-navigation-link", className)}
    />
  );
}

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
    <Primitive.Popup {...props} className={skin("bui-popover", className)} />
  );
}

/** PreviewCard behavior primitives with the Buzz visual roles. */
export const PreviewCard = {
  Root: Primitive.Root,
  Trigger: StyledTrigger,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
};
