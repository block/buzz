import type { ComponentProps } from "react";
import { ScrollArea as Primitive } from "@base-ui/react/scroll-area";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return (
    <Primitive.Root {...props} className={skin("bui-scroll-area", className)} />
  );
}

function StyledViewport({
  className,
  ...props
}: ComponentProps<typeof Primitive.Viewport>) {
  return (
    <Primitive.Viewport
      {...props}
      className={skin("bui-scroll-viewport", className)}
    />
  );
}

function StyledScrollbar({
  className,
  ...props
}: ComponentProps<typeof Primitive.Scrollbar>) {
  return (
    <Primitive.Scrollbar
      {...props}
      className={skin("bui-scrollbar", className)}
    />
  );
}

function StyledThumb({
  className,
  ...props
}: ComponentProps<typeof Primitive.Thumb>) {
  return (
    <Primitive.Thumb
      {...props}
      className={skin("bui-scroll-thumb", className)}
    />
  );
}

function StyledCorner({
  className,
  ...props
}: ComponentProps<typeof Primitive.Corner>) {
  return (
    <Primitive.Corner {...props} className={skin("bui-reset", className)} />
  );
}

/** ScrollArea behavior primitives with the Buzz visual roles. */
export const ScrollArea = {
  Root: StyledRoot,
  Viewport: StyledViewport,
  Scrollbar: StyledScrollbar,
  Thumb: StyledThumb,
  Corner: StyledCorner,
};
