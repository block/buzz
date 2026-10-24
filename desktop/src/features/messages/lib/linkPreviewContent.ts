import type { Node as ProseMirrorNode } from "@tiptap/pm/model";

import { buildPlainTextProjection } from "./plainTextProjection";

export function buildPreviewUpdate(
  doc: ProseMirrorNode,
  selectionAnchor: number,
) {
  const projection = buildPlainTextProjection(doc);
  const plainText = projection.text;
  const hrefs = new Set<string>();
  doc.descendants((node) => {
    if (!node.isText || node.marks.some((mark) => mark.type.name === "spoiler"))
      return;
    const href = node.marks.find((mark) => mark.type.name === "link")?.attrs
      .href;
    if (typeof href === "string") hrefs.add(href);
  });
  return {
    cursor: projection.mapPMToTextOffset(selectionAnchor),
    linkPreviewContent: [plainText, ...hrefs].join("\n"),
    text: plainText,
  };
}
