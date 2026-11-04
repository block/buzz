import {
  formatImetaMediaLine,
  imetaMediaFromTags,
  type ImetaMedia,
} from "@/features/messages/lib/imetaMediaMarkdown";

export const ATTACHMENT_DRAG_MIME = "application/x-buzz-attachment-drag";

export type AttachmentDragItem = ImetaMedia & {
  /** Original imeta tag from the source message, preserved for exact resend. */
  tag: string[];
};

export type AttachmentDragPayload = {
  type: "buzz-attachment-drag";
  version: 1;
  sourceChannelId?: string | null;
  sourceMessageId: string;
  sourceThreadRootId?: string | null;
  sourceAuthor?: string | null;
  items: AttachmentDragItem[];
};

export type AttachmentDragPreview = {
  count: number;
  label: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((entry) => typeof entry === "string")
  );
}

function coerceItem(value: unknown): AttachmentDragItem | null {
  if (!isRecord(value)) return null;
  const { tag, url, type, sha256, size, uploaded } = value;
  if (!isStringArray(tag) || tag[0] !== "imeta") return null;
  if (typeof url !== "string" || url.length === 0) return null;
  if (typeof type !== "string" || type.length === 0) return null;
  if (typeof sha256 !== "string") return null;
  if (typeof size !== "number" || !Number.isFinite(size)) return null;
  if (typeof uploaded !== "number" || !Number.isFinite(uploaded)) return null;

  const item: AttachmentDragItem = {
    tag: [...tag],
    url,
    type,
    sha256,
    size,
    uploaded,
  };

  for (const key of [
    "dim",
    "blurhash",
    "thumb",
    "image",
    "filename",
    "displayLabel",
  ] as const) {
    const field = value[key];
    if (typeof field === "string" && field.length > 0) {
      item[key] = field;
    }
  }

  const duration = value.duration;
  if (typeof duration === "number" && Number.isFinite(duration)) {
    item.duration = duration;
  }

  return item;
}

export function parseAttachmentDragPayload(
  raw: string | null | undefined,
): AttachmentDragPayload | null {
  if (!raw) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isRecord(parsed)) return null;
  if (parsed.type !== "buzz-attachment-drag" || parsed.version !== 1) {
    return null;
  }
  if (typeof parsed.sourceMessageId !== "string" || !parsed.sourceMessageId) {
    return null;
  }
  if (!Array.isArray(parsed.items)) return null;
  const items = parsed.items.flatMap((item) => {
    const coerced = coerceItem(item);
    return coerced ? [coerced] : [];
  });
  if (items.length === 0) return null;

  return {
    type: "buzz-attachment-drag",
    version: 1,
    sourceMessageId: parsed.sourceMessageId,
    sourceChannelId:
      typeof parsed.sourceChannelId === "string" ||
      parsed.sourceChannelId === null
        ? parsed.sourceChannelId
        : undefined,
    sourceThreadRootId:
      typeof parsed.sourceThreadRootId === "string" ||
      parsed.sourceThreadRootId === null
        ? parsed.sourceThreadRootId
        : undefined,
    sourceAuthor:
      typeof parsed.sourceAuthor === "string" || parsed.sourceAuthor === null
        ? parsed.sourceAuthor
        : undefined,
    items,
  };
}

export function serializeAttachmentDragPayload(
  payload: AttachmentDragPayload,
): string {
  return JSON.stringify(payload);
}

export function attachmentDragPreview(
  payload: Pick<AttachmentDragPayload, "items">,
): AttachmentDragPreview {
  const count = payload.items.length;
  const first = payload.items[0];
  const filename = first?.filename?.trim() || first?.displayLabel?.trim();
  return {
    count,
    label: count === 1 ? filename || "1 attachment" : `${count} attachments`,
  };
}

export function attachmentDragItemsFromTags(
  tags: ReadonlyArray<ReadonlyArray<string>> | undefined,
): AttachmentDragItem[] {
  const media = imetaMediaFromTags(tags);
  if (media.length === 0) return [];
  const tagsByUrl = new Map(
    (tags ?? [])
      .filter((tag) => tag[0] === "imeta")
      .flatMap((tag) => {
        const urlPart = tag.find((part) => part.startsWith("url "));
        const url = urlPart?.slice(4);
        return url ? [[url, tag] as const] : [];
      }),
  );

  return media.flatMap((entry) => {
    const tag = tagsByUrl.get(entry.url);
    return tag ? [{ ...entry, tag: [...tag] }] : [];
  });
}

export function attachmentDragContentAndTags(
  payload: Pick<AttachmentDragPayload, "items">,
): { content: string; mediaTags: string[][] } {
  return {
    content: payload.items.map((item) => formatImetaMediaLine(item)).join(""),
    mediaTags: payload.items.map((item) => [...item.tag]),
  };
}
