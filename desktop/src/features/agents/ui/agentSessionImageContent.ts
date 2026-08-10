import type {
  AgentActivityDescriptor,
  TranscriptItem,
} from "./agentSessionTypes";
import { getToolString } from "./agentSessionUtils";

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

export type ImageToolContent = {
  src: string | null;
  localPath: string | null;
  title: string | null;
};

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function remoteOrDataSource(source: string): string | null {
  return source.startsWith("data:image/") ||
    source.startsWith("http://") ||
    source.startsWith("https://")
    ? source
    : null;
}

function localImagePath(source: string): string | null {
  if (/^[a-zA-Z]:[\\/]/.test(source) || source.startsWith("/") || source.startsWith("\\\\")) {
    return source;
  }
  if (source.startsWith("file:///")) {
    try {
      const decoded = decodeURIComponent(source.slice("file:///".length));
      return /^[a-zA-Z]:[\\/]/.test(decoded) ? decoded : `/${decoded}`;
    } catch {
      return null;
    }
  }
  return null;
}

function structuredImageContent(item: ToolItem): ImageToolContent | null {
  for (const value of item.contentBlocks ?? []) {
    const block = asRecord(value);
    const type = asString(block.type)?.toLowerCase();
    if (type === "image") {
      const data = asString(block.data);
      const mimeType =
        asString(block.mimeType) ?? asString(block.mime_type) ?? "image/png";
      if (data && mimeType.startsWith("image/")) {
        return {
          src: data.startsWith("data:image/")
            ? data
            : `data:${mimeType};base64,${data}`,
          localPath: null,
          title: asString(block.name) ?? asString(block.title),
        };
      }
    }
    if (type === "resource_link" || type === "resource") {
      const resource = type === "resource" ? asRecord(block.resource) : block;
      const uri = asString(resource.uri);
      if (!uri) continue;
      const src = remoteOrDataSource(uri);
      const path = src ? null : localImagePath(uri);
      if (src || path) {
        return {
          src,
          localPath: path,
          title:
            asString(resource.name) ??
            asString(resource.title) ??
            asString(block.name) ??
            asString(block.title),
        };
      }
    }
  }
  return null;
}

export function buildImageContent(
  item: ToolItem,
  descriptor: AgentActivityDescriptor,
): ImageToolContent | null {
  if (descriptor.renderClass !== "image") {
    return null;
  }

  const structured = structuredImageContent(item);
  if (structured) {
    return {
      ...structured,
      title:
        structured.title ?? descriptor.preview ?? descriptor.object ?? null,
    };
  }

  const source = getToolString(item.args, ["source", "path"]);
  if (!source) return null;
  const trimmed = source.trim();
  const src = remoteOrDataSource(trimmed);
  const localPath = src ? null : localImagePath(trimmed);
  if (!src && !localPath) return null;

  return {
    src,
    localPath,
    title: descriptor.preview ?? descriptor.object ?? null,
  };
}
