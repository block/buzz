import type { ComponentProps } from "react";
import { Select as Primitive } from "@base-ui/react/select";
import { skin } from "../classes";

function StyledTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.Trigger>) {
  return (
    <Primitive.Trigger
      {...props}
      className={skin("bui-select-trigger", className)}
    />
  );
}

function StyledIcon({
  className,
  ...props
}: ComponentProps<typeof Primitive.Icon>) {
  return <Primitive.Icon {...props} className={skin("bui-icon", className)} />;
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

/** Select behavior primitives with the Buzz visual roles. */
export const Select = {
  Root: Primitive.Root,
  Trigger: StyledTrigger,
  Value: Primitive.Value,
  Icon: StyledIcon,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
  List: StyledList,
  Item: StyledItem,
  ItemText: Primitive.ItemText,
  ItemIndicator: StyledItemIndicator,
  Group: StyledGroup,
  GroupLabel: StyledGroupLabel,
  Separator: StyledSeparator,
};
