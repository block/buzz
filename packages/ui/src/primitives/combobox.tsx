import type { ComponentProps } from "react";
import { Combobox as Primitive } from "@base-ui/react/combobox";
import { skin } from "../classes";

function StyledInput({
  className,
  ...props
}: ComponentProps<typeof Primitive.Input>) {
  return (
    <Primitive.Input {...props} className={skin("bui-input", className)} />
  );
}

function StyledInputGroup({
  className,
  ...props
}: ComponentProps<typeof Primitive.InputGroup>) {
  return (
    <Primitive.InputGroup
      {...props}
      className={skin("bui-input-group", className)}
    />
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

function StyledItemIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.ItemIndicator>) {
  return (
    <Primitive.ItemIndicator
      {...props}
      className={skin("bui-option-check", className)}
    />
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

function StyledGroup({
  className,
  ...props
}: ComponentProps<typeof Primitive.Group>) {
  return (
    <Primitive.Group {...props} className={skin("bui-stack", className)} />
  );
}

function StyledGroupLabel({
  className,
  ...props
}: ComponentProps<typeof Primitive.GroupLabel>) {
  return (
    <Primitive.GroupLabel
      {...props}
      className={skin("bui-group-label", className)}
    />
  );
}

function StyledChips({
  className,
  ...props
}: ComponentProps<typeof Primitive.Chips>) {
  return (
    <Primitive.Chips {...props} className={skin("bui-chips", className)} />
  );
}

function StyledChip({
  className,
  ...props
}: ComponentProps<typeof Primitive.Chip>) {
  return <Primitive.Chip {...props} className={skin("bui-chip", className)} />;
}

function StyledChipRemove({
  className,
  ...props
}: ComponentProps<typeof Primitive.ChipRemove>) {
  return (
    <Primitive.ChipRemove
      {...props}
      className={skin("bui-icon-control", className)}
    />
  );
}

function StyledClear({
  className,
  ...props
}: ComponentProps<typeof Primitive.Clear>) {
  return (
    <Primitive.Clear
      {...props}
      className={skin("bui-icon-control", className)}
    />
  );
}

/** Combobox behavior primitives with the Buzz visual roles. */
export const Combobox = {
  Root: Primitive.Root,
  Input: StyledInput,
  InputGroup: StyledInputGroup,
  Trigger: StyledTrigger,
  Value: Primitive.Value,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
  List: StyledList,
  Item: StyledItem,
  ItemIndicator: StyledItemIndicator,
  Empty: StyledEmpty,
  Group: StyledGroup,
  GroupLabel: StyledGroupLabel,
  Chips: StyledChips,
  Chip: StyledChip,
  ChipRemove: StyledChipRemove,
  Clear: StyledClear,
};
