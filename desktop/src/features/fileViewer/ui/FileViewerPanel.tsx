import * as React from "react";
import { Copy, Download, FileText, Loader2, X } from "lucide-react";
import { toast } from "sonner";

import { copyTextToSystemClipboard } from "@/shared/api/tauriMedia";
import {
  AuxiliaryPanel,
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  type AuxiliaryPanelLayout,
} from "@/shared/layout/AuxiliaryPanel";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Markdown } from "@/shared/ui/markdown";
import { SyntaxHighlightedCode } from "@/shared/ui/markdown/CodeBlock";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import { downloadAttachment } from "../downloadAttachment";
import { classifyFileView } from "../fileViewClassification";
import type { FileViewerContent } from "../fileViewerContent";
import {
  activateFileViewerTab,
  closeFileViewer,
  closeFileViewerTab,
  type FileViewerTab,
  selectActiveFileViewerTab,
} from "../fileViewerStore";
import { useFileViewerContent } from "../useFileViewerContent";
import { useFileViewerState } from "../useFileViewerState";

type FileViewerPanelProps = {
  isSinglePanelView: boolean;
  layout: AuxiliaryPanelLayout;
  transparentChrome?: boolean;
  widthPx: number;
};

const TAB_FOCUS_CLASS =
  "rounded-sm focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring";

/** One chip per open file. This is the panel's only title surface. */
function FileViewerTabStrip({
  activeUrl,
  tabs,
}: {
  activeUrl: string | null;
  tabs: readonly FileViewerTab[];
}) {
  const activeTabRef = React.useRef<HTMLSpanElement | null>(null);
  // The strip scrolls, and no other chrome names the open file, so an activated
  // tab must never stay outside the visible run.
  // biome-ignore lint/correctness/useExhaustiveDependencies: activeUrl triggers the scroll; the effect reads only the ref
  React.useEffect(() => {
    activeTabRef.current?.scrollIntoView({
      block: "nearest",
      inline: "nearest",
    });
  }, [activeUrl]);

  return (
    <div
      className="buzz-file-viewer-tabs-scrollbar flex min-w-0 items-center gap-1 overflow-x-auto"
      data-testid="file-viewer-tab-strip"
    >
      {tabs.map((tab) => {
        const isActive = tab.url === activeUrl;
        return (
          <span
            key={tab.url}
            ref={isActive ? activeTabRef : undefined}
            className={cn(
              "inline-flex max-w-52 shrink-0 items-center gap-1.5 rounded-lg py-1 pl-2 pr-1",
              // Filled, not outlined: the panel surface is already
              // `background`, so an outlined chip reads as a floating box.
              isActive
                ? "bg-muted text-foreground"
                : "text-muted-foreground hover:bg-muted/40",
            )}
          >
            <FileText className="h-3.5 w-3.5 shrink-0 opacity-70" />
            <button
              className={cn(
                "min-w-0 truncate text-xs font-medium",
                TAB_FOCUS_CLASS,
              )}
              data-testid="file-viewer-tab"
              onClick={() => activateFileViewerTab(tab.url)}
              title={tab.filename}
              type="button"
            >
              {tab.filename}
            </button>
            <button
              aria-label={`Close ${tab.filename}`}
              className={cn(
                "shrink-0 p-0.5 text-muted-foreground/70 hover:bg-muted hover:text-foreground",
                TAB_FOCUS_CLASS,
              )}
              data-testid="file-viewer-tab-close"
              onClick={() => closeFileViewerTab(tab.url)}
              type="button"
            >
              <X className="h-3 w-3" />
            </button>
          </span>
        );
      })}
    </div>
  );
}

/** Shown when a file cannot be previewed; download stays available. */
function FileViewerFallback({
  message,
  tab,
}: {
  message: string;
  tab: FileViewerTab;
}) {
  return (
    <div
      className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center"
      data-testid="file-viewer-fallback"
    >
      <span className="flex h-12 w-12 items-center justify-center rounded-xl bg-muted text-muted-foreground">
        <FileText className="h-5 w-5" />
      </span>
      <p className="text-sm text-muted-foreground">{message}</p>
      <Button
        onClick={() => downloadAttachment(tab.url, tab.filename)}
        size="sm"
        type="button"
        variant="secondary"
      >
        <Download />
        Download
      </Button>
    </div>
  );
}

function FileViewerBody({
  content,
  tab,
}: {
  content: FileViewerContent;
  tab: FileViewerTab;
}) {
  if (content.status === "loading") {
    return (
      <div className="flex flex-1 items-center justify-center py-10">
        <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
      </div>
    );
  }
  if (content.status === "error") {
    return <FileViewerFallback message={content.message} tab={tab} />;
  }
  if (content.status === "too-large") {
    return (
      <FileViewerFallback
        message="This file is too large to preview."
        tab={tab}
      />
    );
  }
  if (content.status === "binary") {
    return (
      <FileViewerFallback message="No preview for this file type." tab={tab} />
    );
  }

  const view = classifyFileView(tab.filename, tab.mime);
  if (view.kind === "markdown") {
    return (
      <div className="px-4 py-3" data-testid="file-viewer-markdown">
        <Markdown content={content.text} />
      </div>
    );
  }
  // Shiki degrades to uncolored lines for unknown languages and long files, so
  // plain text can share this path.
  return (
    <pre
      className="m-0 overflow-x-auto px-4 py-3"
      data-testid="file-viewer-code"
    >
      <SyntaxHighlightedCode
        code={content.text}
        language={view.kind === "code" ? view.language : "text"}
      />
    </pre>
  );
}

/** Copies the previewed text; rendered only once the text has loaded. */
function FileViewerCopyAction({ text }: { text: string }) {
  const [isCopying, setIsCopying] = React.useState(false);

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label="Copy file contents"
          data-testid="file-viewer-copy"
          disabled={isCopying}
          onClick={async () => {
            setIsCopying(true);
            try {
              await copyTextToSystemClipboard(text);
              toast.success("Copied file to clipboard");
            } catch (error) {
              console.error("Failed to copy file contents", error);
              toast.error("Failed to copy file");
            } finally {
              setIsCopying(false);
            }
          }}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Copy />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Copy</TooltipContent>
    </Tooltip>
  );
}

/**
 * Right-side panel previewing message attachments, one tab per open file.
 * Mounted by `ChannelPane` in the same resizable shell as the thread and
 * activity panels.
 */
export function FileViewerPanel({
  isSinglePanelView,
  layout,
  transparentChrome = false,
  widthPx,
}: FileViewerPanelProps) {
  const snapshot = useFileViewerState();
  const activeTab = selectActiveFileViewerTab(snapshot);
  // Hoisted above the body so the header's Copy action can reach the text.
  const content = useFileViewerContent(activeTab?.url ?? null, activeTab?.size);

  if (!activeTab) return null;

  return (
    <AuxiliaryPanel
      header={
        <AuxiliaryPanelHeader transparent={transparentChrome}>
          <AuxiliaryPanelHeaderGroup align="center">
            <FileViewerTabStrip
              activeUrl={snapshot.activeUrl}
              tabs={snapshot.tabs}
            />
          </AuxiliaryPanelHeaderGroup>
          <AuxiliaryPanelHeaderActions>
            {content.status === "text" ? (
              <FileViewerCopyAction text={content.text} />
            ) : null}
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Download file"
                  data-testid="file-viewer-download"
                  onClick={() =>
                    downloadAttachment(activeTab.url, activeTab.filename)
                  }
                  size="icon"
                  type="button"
                  variant="ghost"
                >
                  <Download />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Download</TooltipContent>
            </Tooltip>
          </AuxiliaryPanelHeaderActions>
          {/*
           * The docked header floats over the scrolling body, so the rule
           * between tabs and file has to live here — on the body it would
           * scroll away.
           */}
          <span
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 bottom-0 h-px bg-border/70"
            data-testid="file-viewer-header-divider"
          />
        </AuxiliaryPanelHeader>
      }
      isSinglePanelView={isSinglePanelView}
      layout={layout}
      onClose={closeFileViewer}
      testId="file-viewer-panel"
      transparentChrome={transparentChrome}
      widthPx={widthPx}
    >
      <AuxiliaryPanelBody className="overflow-y-auto">
        <FileViewerBody content={content} tab={activeTab} />
      </AuxiliaryPanelBody>
    </AuxiliaryPanel>
  );
}
