import * as React from "react";
import mammoth from "mammoth";

import { sanitizeDocumentHtml } from "@/shared/lib/sanitizeHtml";

/**
 * Converts a .docx file's bytes to HTML via mammoth and renders it.
 *
 * mammoth intentionally does a "best effort" conversion — it maps Word
 * styles to semantic HTML (headings, lists, bold/italic, tables) rather than
 * pixel-perfect layout reproduction. That's the right tradeoff for an in-app
 * preview: readable content over exact fidelity, matching how the markdown
 * and text previews already render.
 */
export function DocxPreview({ bytes }: { bytes: Uint8Array }) {
  const [html, setHtml] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    setHtml(null);
    setError(null);

    // mammoth.convertToHtml wants an exact-length ArrayBuffer; bytes.slice()
    // guarantees a tightly-sized backing buffer even if `bytes` is a view
    // into a larger one.
    const arrayBuffer = bytes.slice().buffer;

    mammoth
      .convertToHtml({ arrayBuffer })
      .then((result) => {
        if (!cancelled) setHtml(result.value);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(
            err instanceof Error ? err.message : "Failed to render document",
          );
        }
      });

    return () => {
      cancelled = true;
    };
  }, [bytes]);

  if (error) {
    return (
      <div className="flex h-full items-center justify-center p-8 text-center text-sm text-muted-foreground">
        {error}
      </div>
    );
  }

  if (html === null) {
    return (
      <div className="flex justify-center py-8 text-sm text-muted-foreground">
        Loading document…
      </div>
    );
  }

  return (
    <div className="p-6">
      {/**
       * mammoth only emits a fixed set of semantic tags (p/headings/lists/
       * table/strong/em/a/img) — it does not pass through arbitrary markup
       * from the .docx XML. It does NOT sanitize attribute values, though: a
       * hyperlink or image src embedded in the source document could still
       * carry a `javascript:`/`data:` URI. sanitizeDocumentHtml (DOMPurify)
       * strips that before render.
       */}
      <div
        className="prose prose-sm dark:prose-invert max-w-none"
        // biome-ignore lint/security/noDangerouslySetInnerHtml: sanitized via DOMPurify above
        dangerouslySetInnerHTML={{ __html: sanitizeDocumentHtml(html) }}
      />
    </div>
  );
}
