import { ToolActivity } from "./ToolActivity";
import type { ActivityRenderClassItemProps } from "./types";

export function SuppressedActivity(props: ActivityRenderClassItemProps) {
  return <ToolActivity {...props} />;
}
