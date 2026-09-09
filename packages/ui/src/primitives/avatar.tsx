import type { ComponentProps } from "react";
import { Avatar as Primitive } from "@base-ui/react/avatar";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return (
    <Primitive.Root {...props} className={skin("bui-avatar", className)} />
  );
}

function StyledImage({
  className,
  ...props
}: ComponentProps<typeof Primitive.Image>) {
  return (
    <Primitive.Image
      {...props}
      className={skin("bui-avatar-image", className)}
    />
  );
}

function StyledFallback({
  className,
  ...props
}: ComponentProps<typeof Primitive.Fallback>) {
  return (
    <Primitive.Fallback
      {...props}
      className={skin("bui-avatar-fallback", className)}
    />
  );
}

/** Avatar behavior primitives with the Buzz visual roles. */
export const Avatar = {
  Root: StyledRoot,
  Image: StyledImage,
  Fallback: StyledFallback,
};
