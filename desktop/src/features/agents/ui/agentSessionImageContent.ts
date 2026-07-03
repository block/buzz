import type {
  AgentActivityDescriptor,
  TranscriptItem,
} from "./agentSessionTypes";
import { getToolString } from "./agentSessionUtils";

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

export type ImageToolContent = {
  src: string;
  title: string | null;
};

export function buildImageContent(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): ImageToolContent | null {
  if (descriptor.renderClass !== "image") {
    return null;
  }

  const source = getToolString(item.args, ["source"]);
  if (!source) {
    return null;
  }

  const trimmed = source.trim();
  if (
    !trimmed.startsWith("data:image/") &&
    !trimmed.startsWith("http://") &&
    !trimmed.startsWith("https://")
  ) {
    return null;
  }

  return {
    src: trimmed,
    title: descriptor.preview ?? descriptor.object ?? null,
  };
}
