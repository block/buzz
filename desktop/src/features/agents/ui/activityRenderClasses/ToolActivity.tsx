import { ToolItem } from "../AgentSessionToolItem";
import type { ActivityRenderClassItemProps } from "./types";

export function ToolActivity(props: ActivityRenderClassItemProps) {
  const { item } = props;
  if (item.type !== "tool") {
    return null;
  }

  return <ToolItem {...props} item={item} />;
}
