import { Group, Panel, Separator } from "react-resizable-panels";
import type { ComponentProps } from "react";
import { cx } from "./classes";
function Handle(props: ComponentProps<typeof Separator>) {
  return (
    <Separator
      {...props}
      className={cx("bui-resize-handle", props.className)}
    />
  );
}
/** Resizable regions with pointer and keyboard support. Sizes use the public panel API. */
export const Resizable = { Group, Panel, Handle };
