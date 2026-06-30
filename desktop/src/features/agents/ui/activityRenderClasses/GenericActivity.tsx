import { ToolActivity } from "./ToolActivity";
import type { ActivityRenderClassItemProps } from "./types";

export function GenericActivity(props: ActivityRenderClassItemProps) {
  return <ToolActivity {...props} />;
}
