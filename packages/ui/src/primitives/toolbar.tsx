import type { ComponentProps } from "react";
import { Toolbar as Primitive } from "@base-ui/react/toolbar";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return (
    <Primitive.Root {...props} className={skin("bui-toolbar", className)} />
  );
}

function StyledGroup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Group>) {
  return (
    <Primitive.Group {...props} className={skin("bui-toolbar", className)} />
  );
}

function StyledButton({
  className,
  ...props
}: ComponentProps<typeof Primitive.Button>) {
  return (
    <Primitive.Button {...props} className={skin("bui-toggle", className)} />
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

function StyledSeparator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Separator>) {
  return (
    <Primitive.Separator
      {...props}
      className={skin("bui-separator", className)}
    />
  );
}

/** Toolbar behavior primitives with the Buzz visual roles. */
export const Toolbar = {
  Root: StyledRoot,
  Group: StyledGroup,
  Button: StyledButton,
  Link: StyledLink,
  Separator: StyledSeparator,
};
