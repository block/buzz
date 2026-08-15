import * as React from "react";
import { pptxToHtml } from "@jvmr/pptx-to-html";

import { sanitizeDocumentHtml } from "@/shared/lib/sanitizeHtml";

/** Standard PowerPoint 4:3 base slide size in CSS px; pptxToHtml rescales to
 * this box regardless of the source deck's actual `sldSz` (16:9 decks letterbox
 * within it via `scaleToFit`/`letterbox` below). */
const SLIDE_WIDTH = 960;
const SLIDE_HEIGHT = 540;

/**
 * Converts a .pptx file's bytes to one HTML snippet per slide via
 * @jvmr/pptx-to-html and renders them stacked, matching how PdfPreview stacks
 * PDF pages.
 *
 * Like mammoth for .docx, fidelity here is intentionally practical, not
 * pixel-perfect: text, images, shapes, tables, and common chart types render;
 * animations, transitions, SmartArt, and embedded video/audio do not. That's
 * the right tradeoff for an in-app preview over pulling a server-side
 * converter (LibreOffice/soffice) into the Docker image.
 */
export function PptxPreview({ bytes }: { bytes: Uint8Array }) {
  const [slides, setSlides] = React.useState<string[] | null>(null);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let cancelled = false;
    setSlides(null);
    setError(null);

    // pptxToHtml wants an exact-length ArrayBuffer; bytes.slice() guarantees
    // a tightly-sized backing buffer even if `bytes` is a view into a larger
    // one (same guard DocxPreview uses before handing bytes to mammoth).
    const arrayBuffer = bytes.slice().buffer;

    pptxToHtml(arrayBuffer, {
      width: SLIDE_WIDTH,
      height: SLIDE_HEIGHT,
      scaleToFit: true,
      letterbox: true,
    })
      .then((html) => {
        if (!cancelled) setSlides(html);
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          setError(
            err instanceof Error
              ? err.message
              : "Failed to render presentation",
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

  if (slides === null) {
    return (
      <div className="flex justify-center py-8 text-sm text-muted-foreground">
        Loading presentation…
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-3 p-4">
      {slides.map((slideHtml, index) => (
        <div
          // Slide order is stable for a given file and a slide carries no
          // independent identity to key on — index is the correct key here,
          // same reasoning as PdfPreview's page loop.
          // biome-ignore lint/suspicious/noArrayIndexKey: stable, order-derived list
          key={index}
          className="mx-auto w-full max-w-3xl overflow-hidden rounded-lg border border-border/60 bg-white shadow-sm"
          style={{ aspectRatio: `${SLIDE_WIDTH} / ${SLIDE_HEIGHT}` }}
          // biome-ignore lint/security/noDangerouslySetInnerHtml: sanitized via DOMPurify above — pptxToHtml does not sanitize attribute values itself
          dangerouslySetInnerHTML={{ __html: sanitizeDocumentHtml(slideHtml) }}
        />
      ))}
    </div>
  );
}
