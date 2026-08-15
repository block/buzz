import * as React from "react";

import {
  checkLibreOfficeAvailable,
  convertPptxToPdf,
} from "@/shared/api/tauriMedia";

import { PdfPreview } from "./PdfPreview";
import { PptxLibreOfficePrompt } from "./PptxLibreOfficePrompt";
import { PptxPreview } from "./PptxPreview";

type Stage =
  | { kind: "checking" }
  | { kind: "converting" }
  | { kind: "pdf"; bytes: Uint8Array }
  | { kind: "unavailable"; errorDetail: string | null }
  | { kind: "basic" };

/**
 * `.pptx` preview entry point: prefers a pixel-accurate LibreOffice-rendered
 * PDF over the client-side JS renderer (`PptxPreview`) when LibreOffice is
 * available on this machine, and falls back gracefully (with an explicit
 * "install LibreOffice" prompt, not a silent downgrade) when it isn't.
 *
 * State machine:
 *  1. `checking` — call `check_libreoffice_available` on mount.
 *  2. If available → `converting` — call `convert_pptx_to_pdf`, then render
 *     the result through the existing `PdfPreview`.
 *     - A runtime conversion failure (LibreOffice reported available but
 *       errored on this specific file) is treated the same as "not
 *       available" rather than crashing the preview, with the error message
 *       kept for an optional "details" affordance.
 *  3. If not available → `unavailable` — show `PptxLibreOfficePrompt`, which
 *     offers "Download LibreOffice", "Retry" (re-runs step 1, so a
 *     just-installed LibreOffice is picked up without restarting the app),
 *     and "View basic preview instead" (→ `basic`, the plain JS renderer).
 */
export function PptxSmartPreview({ bytes }: { bytes: Uint8Array }) {
  const [stage, setStage] = React.useState<Stage>({ kind: "checking" });
  const [retrying, setRetrying] = React.useState(false);

  const runCheck = React.useCallback((bytesToConvert: Uint8Array) => {
    let cancelled = false;
    setStage({ kind: "checking" });

    checkLibreOfficeAvailable()
      .then((available) => {
        if (cancelled) return;
        if (!available) {
          setStage({ kind: "unavailable", errorDetail: null });
          return;
        }
        setStage({ kind: "converting" });
        return convertPptxToPdf(bytesToConvert)
          .then((pdfBytes) => {
            if (cancelled) return;
            setStage({ kind: "pdf", bytes: pdfBytes });
          })
          .catch((err: unknown) => {
            if (cancelled) return;
            const detail =
              err instanceof Error ? err.message : "Conversion failed";
            setStage({ kind: "unavailable", errorDetail: detail });
          });
      })
      .catch(() => {
        if (cancelled) return;
        setStage({ kind: "unavailable", errorDetail: null });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  React.useEffect(() => runCheck(bytes), [bytes, runCheck]);

  const handleRetry = React.useCallback(() => {
    setRetrying(true);
    checkLibreOfficeAvailable()
      .then((available) => {
        setRetrying(false);
        if (available) {
          runCheck(bytes);
        }
      })
      .catch(() => setRetrying(false));
  }, [bytes, runCheck]);

  const handleViewBasicPreview = React.useCallback(() => {
    setStage({ kind: "basic" });
  }, []);

  switch (stage.kind) {
    case "checking":
      return (
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          Checking for LibreOffice…
        </div>
      );
    case "converting":
      return (
        <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
          Rendering presentation…
        </div>
      );
    case "pdf":
      return <PdfPreview bytes={stage.bytes} />;
    case "basic":
      return <PptxPreview bytes={bytes} />;
    case "unavailable":
      return (
        <PptxLibreOfficePrompt
          errorDetail={stage.errorDetail}
          onRetry={handleRetry}
          onViewBasicPreview={handleViewBasicPreview}
          retrying={retrying}
        />
      );
    default:
      return null;
  }
}
