import type { ComponentProps } from "react";
import { Accordion as Primitive } from "@base-ui/react/accordion";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-stack", className)} />;
}

function StyledItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.Item>) {
  return (
    <Primitive.Item
      {...props}
      className={skin("bui-accordion-item", className)}
    />
  );
}

function StyledHeader({
  className,
  ...props
}: ComponentProps<typeof Primitive.Header>) {
  return (
    <Primitive.Header {...props} className={skin("bui-reset", className)} />
  );
}

function StyledTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.Trigger>) {
  return (
    <Primitive.Trigger
      {...props}
      className={skin("bui-accordion-trigger", className)}
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

/** Accordion behavior primitives with the Buzz visual roles. */
export const Accordion = {
  Root: StyledRoot,
  Item: StyledItem,
  Header: StyledHeader,
  Trigger: StyledTrigger,
  Panel: StyledPanel,
};
