import type { ComponentProps } from "react";
import { Drawer as Primitive } from "@base-ui/react/drawer";
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

function StyledViewport({
  className,
  ...props
}: ComponentProps<typeof Primitive.Viewport>) {
  return (
    <Primitive.Viewport
      {...props}
      className={skin("bui-drawer-viewport", className)}
    />
  );
}

function StyledPopup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Popup>) {
  return (
    <Primitive.Popup {...props} className={skin("bui-drawer", className)} />
  );
}

function StyledContent({
  className,
  ...props
}: ComponentProps<typeof Primitive.Content>) {
  return (
    <Primitive.Content {...props} className={skin("bui-stack", className)} />
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

/** Drawer behavior primitives with the Buzz visual roles. */
export const Drawer = {
  Root: Primitive.Root,
  Trigger: Primitive.Trigger,
  Portal: Primitive.Portal,
  Backdrop: StyledBackdrop,
  Viewport: StyledViewport,
  Popup: StyledPopup,
  Content: StyledContent,
  Title: StyledTitle,
  Description: StyledDescription,
  Close: Primitive.Close,
};
