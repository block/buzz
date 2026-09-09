import type { ComponentProps } from "react";
import { cx } from "./classes";
/** Inline input accessories. Label the input itself; decorative accessories should be aria-hidden. */
export function InputGroup(props: ComponentProps<"div">) {
  return <div {...props} className={cx("bui-input-group", props.className)} />;
}
