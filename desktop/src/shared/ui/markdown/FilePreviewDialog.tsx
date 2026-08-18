import * as React from "react";
import { Download, Loader2 } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import { fetchMediaBytes } from "@/shared/api/tauriMedia";
import {
  type AttachmentPreviewKind,
  decodeTextPreview,
  MAX_TEXT_PREVIEW_BYTES,
} from "@/shared/ui/attachmentPreview";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import { SyntaxHighlightedCode } from "./CodeBlock";

type FilePreviewDialogProps = {
  filename: string;
  href: string;
  kind: Exclude<AttachmentPreviewKind, { kind: "none" }>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  renderMarkdown: (content: string) => React.ReactNode;
  size?: number;
  sizeLabel: string;
};

type PreviewState =
  | { status: "idle" | "loading" }
  | { status: "error"; message: string }
  | { status: "text"; content: string }
  | { status: "pdf"; objectUrl: string };

function errorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Unable to preview this file.";
}

export function FilePreviewDialog({
  filename,
  href,
  kind,
  onOpenChange,
  open,
  renderMarkdown,
  size,
  sizeLabel,
}: FilePreviewDialogProps) {
  const [state, setState] = React.useState<PreviewState>({ status: "idle" });

  React.useEffect(() => {
    if (!open) {
      setState((current) => {
        if (current.status === "pdf") URL.revokeObjectURL(current.objectUrl);
        return { status: "idle" };
      });
      return;
    }

    let cancelled = false;
    let objectUrl: string | undefined;

    async function loadPreview() {
      if (
        kind.kind !== "pdf" &&
        size != null &&
        size > MAX_TEXT_PREVIEW_BYTES
      ) {
        setState({
          status: "error",
          message: "This file is too large to preview (2 MB maximum).",
        });
        return;
      }

      setState({ status: "loading" });
      try {
        const bytes = await fetchMediaBytes(href);
        if (cancelled) return;

        if (kind.kind === "pdf") {
          objectUrl = URL.createObjectURL(
            new Blob([bytes], { type: "application/pdf" }),
          );
          setState({ status: "pdf", objectUrl });
          return;
        }

        setState({ status: "text", content: decodeTextPreview(bytes) });
      } catch (error) {
        if (!cancelled)
          setState({ status: "error", message: errorMessage(error) });
      }
    }

    void loadPreview();
    return () => {
      cancelled = true;
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [href, kind.kind, open, size]);

  const download = React.useCallback(() => {
    invokeTauri("download_file", { url: href, filename }).catch(
      (error: unknown) => {
        toast.error(
          errorMessage(error).replace(
            "Unable to preview",
            "Unable to download",
          ),
        );
      },
    );
  }, [filename, href]);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="flex h-[80vh] w-full max-w-5xl flex-col gap-0 overflow-hidden p-0"
        data-testid="file-preview-dialog"
      >
        <DialogHeader className="shrink-0 border-b border-border/70 px-5 py-4 pr-24">
          <DialogTitle className="truncate text-base">{filename}</DialogTitle>
          <DialogDescription>
            {sizeLabel || "Attachment preview"}
          </DialogDescription>
        </DialogHeader>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="Download file"
              className="absolute right-14 top-4"
              data-testid="file-preview-download"
              onClick={download}
              size="icon"
              type="button"
              variant="ghost"
            >
              <Download />
            </Button>
          </TooltipTrigger>
          <TooltipContent>Download</TooltipContent>
        </Tooltip>

        <div className="min-h-0 flex-1 overflow-auto bg-muted/20">
          {state.status === "idle" || state.status === "loading" ? (
            <div className="flex h-full items-center justify-center text-muted-foreground">
              <Loader2 className="h-5 w-5 animate-spin" />
              <span className="ml-2 text-sm">Loading preview...</span>
            </div>
          ) : null}

          {state.status === "error" ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
              <p className="text-sm text-muted-foreground">{state.message}</p>
              <Button
                onClick={download}
                size="sm"
                type="button"
                variant="outline"
              >
                <Download />
                Download file
              </Button>
            </div>
          ) : null}

          {state.status === "pdf" ? (
            <iframe
              className="h-full min-h-[24rem] w-full border-0 bg-background"
              src={state.objectUrl}
              title={`${filename} PDF preview`}
            />
          ) : null}

          {state.status === "text" && kind.kind === "markdown" ? (
            <div className="mx-auto max-w-4xl px-6 py-5">
              {renderMarkdown(state.content)}
            </div>
          ) : null}

          {state.status === "text" && kind.kind === "text" ? (
            <pre className="min-h-full min-w-full overflow-visible p-5">
              {kind.language ? (
                <SyntaxHighlightedCode
                  code={state.content}
                  language={kind.language}
                />
              ) : (
                <code className="block min-w-full whitespace-pre font-mono text-sm text-foreground">
                  {state.content}
                </code>
              )}
            </pre>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
