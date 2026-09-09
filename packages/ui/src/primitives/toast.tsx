import type { ComponentProps } from "react";
import { Toast as Primitive } from "@base-ui/react/toast";
import { skin } from "../classes";

function StyledViewport({
  className,
  ...props
}: ComponentProps<typeof Primitive.Viewport>) {
  return (
    <Primitive.Viewport
      {...props}
      className={skin("bui-toast-viewport", className)}
    />
  );
}

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-toast", className)} />;
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
    <Primitive.Title {...props} className={skin("bui-label", className)} />
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

function StyledClose({
  className,
  ...props
}: ComponentProps<typeof Primitive.Close>) {
  return (
    <Primitive.Close
      aria-hidden={false}
      {...props}
      className={skin("bui-icon-control", className)}
    />
  );
}

function StyledAction({
  className,
  ...props
}: ComponentProps<typeof Primitive.Action>) {
  return (
    <Primitive.Action
      {...props}
      className={skin("bui-disclosure", className)}
    />
  );
}

/** Toast behavior primitives with the Buzz visual roles. */
export const Toast = {
  Provider: Primitive.Provider,
  Portal: Primitive.Portal,
  Viewport: StyledViewport,
  Root: StyledRoot,
  Content: StyledContent,
  Title: StyledTitle,
  Description: StyledDescription,
  Close: StyledClose,
  Action: StyledAction,
  useToastManager: Primitive.useToastManager,
  createToastManager: Primitive.createToastManager,
};
