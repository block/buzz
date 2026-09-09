import type { ComponentProps } from "react";
import { AlertDialog as Primitive } from "@base-ui/react/alert-dialog";
import { skin } from "../classes";

function StyledBackdrop({
  className,
  ...props
}: ComponentProps<typeof Primitive.Backdrop>) {
  return (
    <Primitive.Backdrop
      {...props}
      className={skin("bui-backdrop", className)}
    />
  );
}

function StyledPopup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Popup>) {
  return (
    <Primitive.Popup {...props} className={skin("bui-dialog", className)} />
  );
}

function StyledTitle({
  className,
  ...props
}: ComponentProps<typeof Primitive.Title>) {
  return (
    <Primitive.Title {...props} className={skin("bui-title", className)} />
  );
}

function StyledDescription({
  className,
  ...props
}: ComponentProps<typeof Primitive.Description>) {
  return (
    <Primitive.Description
      {...props}
      className={skin("bui-description", className)}
    />
  );
}

/** AlertDialog behavior primitives with the Buzz visual roles. */
export const AlertDialog = {
  Root: Primitive.Root,
  Trigger: Primitive.Trigger,
  Portal: Primitive.Portal,
  Backdrop: StyledBackdrop,
  Popup: StyledPopup,
  Title: StyledTitle,
  Description: StyledDescription,
  Close: Primitive.Close,
};
