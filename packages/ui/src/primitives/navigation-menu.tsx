import type { ComponentProps } from "react";
import { NavigationMenu as Primitive } from "@base-ui/react/navigation-menu";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return (
    <Primitive.Root {...props} className={skin("bui-navigation", className)} />
  );
}

function StyledList({
  className,
  ...props
}: ComponentProps<typeof Primitive.List>) {
  return (
    <Primitive.List {...props} className={skin("bui-toolbar", className)} />
  );
}

function StyledItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.Item>) {
  return <Primitive.Item {...props} className={skin("bui-reset", className)} />;
}

function StyledTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.Trigger>) {
  return (
    <Primitive.Trigger
      {...props}
      className={skin("bui-disclosure", className)}
    />
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

function StyledLink({
  className,
  ...props
}: ComponentProps<typeof Primitive.Link>) {
  return (
    <Primitive.Link
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
    <Primitive.Popup {...props} className={skin("bui-popup", className)} />
  );
}

function StyledViewport({
  className,
  ...props
}: ComponentProps<typeof Primitive.Viewport>) {
  return (
    <Primitive.Viewport {...props} className={skin("bui-stack", className)} />
  );
}

/** NavigationMenu behavior primitives with the Buzz visual roles. */
export const NavigationMenu = {
  Root: StyledRoot,
  List: StyledList,
  Item: StyledItem,
  Trigger: StyledTrigger,
  Content: StyledContent,
  Link: StyledLink,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
  Viewport: StyledViewport,
};
