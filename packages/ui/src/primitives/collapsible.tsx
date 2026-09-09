import type { ComponentProps } from "react";
import { Collapsible as Primitive } from "@base-ui/react/collapsible";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-stack", className)} />;
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

function StyledPanel({
  className,
  ...props
}: ComponentProps<typeof Primitive.Panel>) {
  return (
    <Primitive.Panel
      {...props}
      className={skin("bui-accordion-panel", className)}
    />
  );
}

/** Collapsible behavior primitives with the Buzz visual roles. */
export const Collapsible = {
  Root: StyledRoot,
  Trigger: StyledTrigger,
  Panel: StyledPanel,
};
