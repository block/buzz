import type { ComponentProps } from "react";
import { Tabs as Primitive } from "@base-ui/react/tabs";
import { skin } from "../classes";

function StyledRoot({
  className,
  ...props
}: ComponentProps<typeof Primitive.Root>) {
  return <Primitive.Root {...props} className={skin("bui-stack", className)} />;
}

function StyledList({
  variant = "solid",
  className,
  ...props
}: ComponentProps<typeof Primitive.List> & {
  /** Material of the tab track; behavior and indicator motion remain shared. */
  variant?: "solid" | "glass";
}) {
  return (
    <Primitive.List
      {...props}
      data-variant={variant}
      className={skin("bui-tabs-list", className)}
    />
  );
}

function StyledTab({
  className,
  ...props
}: ComponentProps<typeof Primitive.Tab>) {
  return <Primitive.Tab {...props} className={skin("bui-tab", className)} />;
}

function StyledPanel({
  className,
  ...props
}: ComponentProps<typeof Primitive.Panel>) {
  return (
    <Primitive.Panel {...props} className={skin("bui-tab-panel", className)} />
  );
}

function StyledIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.Indicator>) {
  return (
    <Primitive.Indicator
      {...props}
      className={skin("bui-tab-indicator", className)}
    />
  );
}

/** Tabs behavior primitives with the Buzz visual roles. */
export const Tabs = {
  Root: StyledRoot,
  List: StyledList,
  Tab: StyledTab,
  Panel: StyledPanel,
  Indicator: StyledIndicator,
};
