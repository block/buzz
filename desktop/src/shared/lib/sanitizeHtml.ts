import DOMPurify from "dompurify";

/**
 * Sanitizes HTML produced by client-side document parsers (mammoth for
 * .docx, @jvmr/pptx-to-html for .pptx) before it is injected via
 * `dangerouslySetInnerHTML`.
 *
 * Both parsers convert attacker-controlled document XML into HTML but do not
 * sanitize attribute values themselves — a hyperlink `href`, image `src`, or
 * inline `style` embedded in the source document could otherwise carry a
 * `javascript:`/`data:` URI or a `<script>`/event-handler payload. DOMPurify
 * strips those by default (no custom config needed) while leaving the
 * semantic markup these parsers emit (headings, lists, tables, images,
 * absolutely-positioned slide content) intact.
 */
export function sanitizeDocumentHtml(html: string): string {
  return DOMPurify.sanitize(html);
}
