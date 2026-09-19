import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { Download, FileText, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import {
  fetchMarkdownDocBytes,
  isMediaTooLargeError,
} from "@/shared/api/tauriMedia";
import {
  focusMarkdownDocPanelClose,
  restoreFocusToMarkdownDocOpener,
} from "@/features/channels/ui/markdownDocFocus";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { cn } from "@/shared/lib/cn";
import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelHeaderTitleBlock,
} from "@/shared/layout/AuxiliaryPanel";
import { Button } from "@/shared/ui/button";
import { Markdown, SyntaxHighlightedCode } from "@/shared/ui/markdown";
import {
  decodeMarkdownDocBytes,
  type MarkdownDocDecodeResult,
} from "@/shared/ui/markdown/markdownDocFile";
import { SegmentedControl } from "@/shared/ui/segmented-control";

type MarkdownDocView = "preview" | "code";

type MarkdownDocPanelProps = {
  /** Raw relay `/media/` URL of the attachment. */
  url: string;
  /** Human-readable filename from the imeta `filename` field. */
  filename: string;
  /** Fill a parent focus drawer and center the document reading column. */
  isFocusMode?: boolean;
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  onClose: () => void;
  transparentChrome?: boolean;
  widthPx: number;
};

const VIEW_OPTIONS = [
  { value: "preview", label: "Preview" },
  { value: "code", label: "Code" },
] as const;

function decodeErrorMessage(kind: "too-large" | "binary"): string {
  return kind === "too-large"
    ? "This file is too large to preview."
    : "This file isn't valid text, so it can't be previewed.";
}

/**
 * Right auxiliary panel rendering a shared markdown attachment in-app.
 *
 * Relay media URLs require relay auth (plain browser requests 401), so the
 * content is fetched through the authenticated `fetch_markdown_doc_bytes`
 * Tauri command — which enforces the viewer's 2 MiB cap natively during the
 * fetch — and rendered with the same markdown pipeline chat messages use.
 * The Preview/Code toggle switches between rendered markdown and the
 * syntax-highlighted source.
 */
export function MarkdownDocPanel({
  url,
  filename,
  isFocusMode = false,
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  transparentChrome = false,
  widthPx,
}: MarkdownDocPanelProps) {
  const isOverlay = useIsThreadPanelOverlay();
  useEscapeKey(onClose, isOverlay || isSinglePanelView);
  const [view, setView] = React.useState<MarkdownDocView>("preview");

  // Opening can unmount the section holding the focused attachment card
  // (narrow layout swaps the whole channel out), and closing unmounts this
  // panel — move focus in on mount and hand it back to the opener card on
  // unmount so keyboard users never fall to <body>.
  React.useEffect(() => {
    const cancel = focusMarkdownDocPanelClose();
    return () => {
      cancel();
      restoreFocusToMarkdownDocOpener(url);
    };
  }, [url]);

  // Blob URLs are content-addressed (`/media/{sha256}.{ext}`), so a fetched
  // document never changes under its URL — cache it for the session.
  const docQuery = useQuery<MarkdownDocDecodeResult>({
    queryKey: ["markdown-doc", url],
    queryFn: async () => {
      try {
        return decodeMarkdownDocBytes(await fetchMarkdownDocBytes(url));
      } catch (err) {
        // The native 2 MiB cap refuses oversized documents during the fetch
        // (the in-frontend decode check never sees their bytes). Surface it
        // as the too-large fallback rather than a generic fetch failure.
        if (isMediaTooLargeError(err)) return { kind: "too-large" };
        throw err;
      }
    },
    staleTime: Number.POSITIVE_INFINITY,
    retry: 1,
  });

  const handleDownload = React.useCallback(() => {
    invokeTauri("download_file", { url, filename }).catch((err: unknown) => {
      const msg = err instanceof Error ? err.message : "Download failed";
      toast.error(msg);
    });
  }, [url, filename]);

  const decoded = docQuery.data;
  const errorMessage = docQuery.isError
    ? "Couldn't load this file from the relay."
    : decoded && decoded.kind !== "ok"
      ? decodeErrorMessage(decoded.kind)
      : null;

  return (
    <AuxiliaryPanel
      className={isFocusMode ? "w-full" : undefined}
      isSinglePanelView={isFocusMode || isSinglePanelView}
      layout={layout}
      onClose={onClose}
      testId="markdown-doc-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
      header={
        <AuxiliaryPanelHeader
          backdrop={layout !== "split" && !isOverlay}
          backdropSurface="soft"
          inset={layout !== "split" ? "wide" : "default"}
        >
          <AuxiliaryPanelHeaderGroup>
            <FileText className="h-4 w-4 shrink-0 text-muted-foreground" />
            <AuxiliaryPanelHeaderTitleBlock title={filename} />
          </AuxiliaryPanelHeaderGroup>
          <AuxiliaryPanelHeaderActions includeCloseAction>
            <Button
              aria-label={`Download ${filename}`}
              data-testid="markdown-doc-download"
              onClick={handleDownload}
              size="icon"
              variant="ghost"
            >
              <Download className="h-4 w-4" />
            </Button>
          </AuxiliaryPanelHeaderActions>
        </AuxiliaryPanelHeader>
      }
    >
      <AuxiliaryPanelBody
        className="flex min-h-0 flex-col"
        panelPadding={!isFocusMode}
      >
        {/* The view picker gets its own pinned row below the title: sharing
            the title row squeezed the filename out, and the header chrome
            band overlays anything placed directly after it in the header
            slot — so the row lives inside the chrome-padded body instead. */}
        {decoded?.kind === "ok" ? (
          <div
            className={cn(
              "flex w-full shrink-0 items-center px-4 pb-2",
              isFocusMode && "mx-auto max-w-[55rem]",
            )}
          >
            <SegmentedControl
              legend="Document view"
              onValueChange={setView}
              optionTestIdPrefix="markdown-doc-view"
              options={VIEW_OPTIONS}
              size="compact"
              testId="markdown-doc-view-toggle"
              value={view}
            />
          </div>
        ) : null}
        <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-6">
          <div className={cn("w-full", isFocusMode && "mx-auto max-w-[55rem]")}>
            {docQuery.isPending ? (
              <div
                className="flex items-center justify-center py-12"
                data-testid="markdown-doc-loading"
              >
                <Loader2 className="h-5 w-5 animate-spin text-muted-foreground/70" />
              </div>
            ) : errorMessage !== null ? (
              <div className="flex flex-col items-center gap-3 py-12 text-center">
                <p className="text-sm text-muted-foreground">{errorMessage}</p>
                <Button onClick={handleDownload} size="sm" variant="secondary">
                  <Download className="mr-1.5 h-4 w-4" />
                  Download file
                </Button>
              </div>
            ) : decoded?.kind === "ok" ? (
              view === "preview" ? (
                <Markdown
                  blockCode
                  className="pt-3 text-sm"
                  content={decoded.text}
                  hardLineBreaks={false}
                />
              ) : (
                <pre
                  className="overflow-x-auto pt-3 text-xs leading-relaxed"
                  data-testid="markdown-doc-code"
                >
                  {/* Shiki's synchronous-tokenization guard caps highlighting at
                  150 lines; longer documents render as plain text here. */}
                  <SyntaxHighlightedCode
                    code={decoded.text}
                    language="markdown"
                  />
                </pre>
              )
            ) : null}
          </div>
        </div>
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}
