import type { ComponentProps } from "react";
import { ContextMenu as Primitive } from "@base-ui/react/context-menu";
import { skin } from "../classes";

function StyledTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.Trigger>) {
  return (
    <Primitive.Trigger
      {...props}
      className={skin("bui-context-target", className)}
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

function StyledItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.Item>) {
  return (
    <Primitive.Item {...props} className={skin("bui-option", className)} />
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

function StyledCheckboxItem({
  className,
  ...props
}: ComponentProps<typeof Primitive.CheckboxItem>) {
  return (
    <Primitive.CheckboxItem
      {...props}
      className={skin("bui-option", className)}
    />
  );
}

function StyledCheckboxItemIndicator({
  className,
  ...props
}: ComponentProps<typeof Primitive.CheckboxItemIndicator>) {
  return (
    <Primitive.CheckboxItemIndicator
      {...props}
      className={skin("bui-option-check", className)}
    />
  );
}

function StyledSubmenuTrigger({
  className,
  ...props
}: ComponentProps<typeof Primitive.SubmenuTrigger>) {
  return (
    <Primitive.SubmenuTrigger
      {...props}
      className={skin("bui-option", className)}
    />
  );
}

/** ContextMenu behavior primitives with the Buzz visual roles. */
export const ContextMenu = {
  Root: Primitive.Root,
  Trigger: StyledTrigger,
  Portal: Primitive.Portal,
  Positioner: StyledPositioner,
  Popup: StyledPopup,
  Item: StyledItem,
  Separator: StyledSeparator,
  GroupLabel: StyledGroupLabel,
  CheckboxItem: StyledCheckboxItem,
  CheckboxItemIndicator: StyledCheckboxItemIndicator,
  SubmenuRoot: Primitive.SubmenuRoot,
  SubmenuTrigger: StyledSubmenuTrigger,
};
