import { ToolItem } from "../AgentSessionToolItem";
import type { ActivityRenderClassItemProps } from "./types";

export function ToolActivity({ item }: ActivityRenderClassItemProps) {
  if (item.type !== "tool") {
    return null;
  }

  return <ToolItem item={item} />;
}
