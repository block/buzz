import { ToolActivity } from "./ToolActivity";
import type { ActivityRenderClassItemProps } from "./types";

export function ShellActivity(props: ActivityRenderClassItemProps) {
  return <ToolActivity {...props} />;
}
