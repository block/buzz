import type { ComponentProps } from "react";
import { Autocomplete as Primitive } from "@base-ui/react/autocomplete";
import { skin } from "../classes";

function StyledInput({
  className,
  ...props
}: ComponentProps<typeof Primitive.Input>) {
  return (
    <Primitive.Input {...props} className={skin("bui-input", className)} />
  );
}

function StyledTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.Trigger>) {
  return (
    <Primitive.Trigger
      {...props}
      className={skin("bui-icon-control", className)}
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

function StyledList({
  className,
  ...props
}: ComponentProps<typeof Primitive.List>) {
  return (
    <Primitive.List {...props} className={skin("bui-option-list", className)} />
  );
}

function StyledItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.Item>) {
  return (
    <Primitive.Item {...props} className={skin("bui-option", className)} />
  );
}

function StyledEmpty({
  className,
  ...props
}: ComponentProps<typeof Primitive.Empty>) {
  return (
    <Primitive.Empty {...props} className={skin("bui-empty", className)} />
  );
}

/** Autocomplete behavior primitives with the Buzz visual roles. */
export const Autocomplete = {
  Root: Primitive.Root,
  Input: StyledInput,
  Trigger: StyledTrigger,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
  List: StyledList,
  Item: StyledItem,
  Empty: StyledEmpty,
};
