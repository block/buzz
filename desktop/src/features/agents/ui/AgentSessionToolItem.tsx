import * as React from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { ChevronDown, Send } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { AgentActivityAction, TranscriptItem } from "./agentSessionTypes";
import { getBuzzToolInfo } from "./agentSessionToolCatalog";
import {
  buildCompactToolSummary,
  type CompactFileEditSummary,
} from "./agentSessionToolSummary";
import type {
  FileEditDiff,
  FileEditDiffLine,
} from "./agentSessionFileEditDiff";
import {
  ActivityRowLabel,
  splitActivityRowLabel,
  type ActivityRowLabelParts,
} from "./activityRenderClasses/ActivityRow";
import {
  formatCodeValue,
  getToolDurationDisplay,
  isInlineImageData,
} from "./agentSessionUtils";

export function ToolItem({
  item,
}: {
  item: Extract<TranscriptItem, { type: "tool" }>;
}) {
  const [isExpanded, setIsExpanded] = React.useState(false);
  const hasArgs = Object.keys(item.args).length > 0;
  const hasResult = item.result.trim().length > 0;
  const canonicalToolName = item.buzzToolName ?? item.toolName;
  const buzzTool = getBuzzToolInfo(canonicalToolName);
  const compactSummary = buildCompactToolSummary(item);
  const duration = getToolDurationDisplay(item);
  const handleToggle = React.useCallback(
    (event: React.SyntheticEvent<HTMLDetailsElement>) => {
      setIsExpanded(event.currentTarget.open);
    },
    [],
  );

  return (
    <div className="not-prose w-full" data-testid="transcript-tool-item">
      <details
        className="group w-full"
        onToggle={handleToggle}
        open={isExpanded}
      >
        <summary
          className={cn(
            "max-w-full cursor-pointer list-none",
            compactSummary.presentation === "inline" &&
              "inline-flex items-center gap-1.5",
            compactSummaryTone(),
          )}
        >
          {compactSummary.presentation === "message" ? (
            <CompactMessageSummary
              duration={duration}
              isError={item.isError || item.status === "failed"}
              label={compactSummary.label}
              preview={compactSummary.preview}
            />
          ) : (
            <CompactToolSummaryRow
              action={compactSummary.action}
              duration={duration}
              fileEditSummary={compactSummary.fileEditSummary}
              preview={compactSummary.preview}
              thumbnailSrc={compactSummary.thumbnailSrc}
              label={compactSummary.label}
            />
          )}
        </summary>

        <ToolDetailBlocks
          args={item.args}
          description={buzzTool?.label}
          fileEditDiff={compactSummary.fileEditDiff}
          hasArgs={hasArgs}
          hasResult={hasResult}
          imagePreview={
            compactSummary.thumbnailSrc != null && isExpanded
              ? {
                  src: compactSummary.thumbnailSrc,
                  title: compactSummary.preview,
                }
              : null
          }
          isError={item.isError}
          result={item.result}
        />
      </details>
    </div>
  );
}

function compactSummaryTone() {
  return "text-muted-foreground/60 group-open:text-muted-foreground";
}

function resolveImageSrc(source: string): string {
  return isInlineImageData(source) ? source : rewriteRelayUrl(source);
}

function CompactToolSummaryRow({
  action,
  duration,
  fileEditSummary,
  label,
  preview,
  thumbnailSrc,
}: {
  action: AgentActivityAction | null;
  duration: string | null;
  fileEditSummary: CompactFileEditSummary | null;
  label: string;
  preview: string | null;
  thumbnailSrc: string | null;
}) {
  const [thumbnailFailed, setThumbnailFailed] = React.useState(false);
  const mutedTone = compactSummaryTone();
  const resolvedThumbnail = React.useMemo(() => {
    if (!thumbnailSrc || thumbnailFailed) return null;
    return resolveImageSrc(thumbnailSrc);
  }, [thumbnailFailed, thumbnailSrc]);
  const actionLabel = fileEditSummary
    ? null
    : getCompactToolActionLabel(action, label, preview);

  return (
    <>
      {fileEditSummary ? (
        <CompactFileEditSummaryView summary={fileEditSummary} />
      ) : actionLabel ? (
        <ActivityRowLabel
          object={actionLabel.object}
          openToneScope="tool"
          title={actionLabel.title}
          verb={actionLabel.verb}
        />
      ) : (
        <span className={cn("shrink-0 text-sm font-semibold", mutedTone)}>
          {label}
        </span>
      )}
      {!fileEditSummary && resolvedThumbnail ? (
        <img
          alt=""
          className="h-5 w-auto max-w-12 shrink-0 rounded-sm object-cover"
          decoding="async"
          loading="lazy"
          onError={() => setThumbnailFailed(true)}
          src={resolvedThumbnail}
          title={preview ?? undefined}
        />
      ) : !fileEditSummary && !actionLabel && preview ? (
        <span
          className={cn("min-w-0 max-w-48 truncate text-sm", mutedTone)}
          title={preview}
        >
          {preview}
        </span>
      ) : null}
      {duration ? (
        <span className={cn("shrink-0 text-xs", mutedTone)}>{duration}</span>
      ) : null}
      <ChevronDown
        className={cn(
          "h-3.5 w-3.5 shrink-0 transition-transform group-open:rotate-180",
          mutedTone,
        )}
      />
    </>
  );
}

function getCompactToolActionLabel(
  action: AgentActivityAction | null,
  label: string,
  preview: string | null,
): (ActivityRowLabelParts & { title?: string }) | null {
  if (action) {
    const object = action.object ?? preview ?? undefined;
    return {
      verb: action.verb,
      object,
      title: typeof object === "string" ? object : undefined,
    };
  }

  const parts = splitActivityRowLabel(label);
  if (!parts) return null;

  if (!preview) return parts;

  if (
    label === "Ran command" ||
    label === "Read file" ||
    label === "Updated todos" ||
    label === "Viewed image"
  ) {
    return { verb: parts.verb, object: preview, title: preview };
  }

  return parts;
}

function CompactFileEditSummaryView({
  summary,
}: {
  summary: CompactFileEditSummary;
}) {
  return (
    <ActivityRowLabel
      className="max-w-72"
      object={summary.filename}
      openToneScope="tool"
      stats={{
        additions: summary.additions,
        deletions: summary.deletions,
      }}
      title={summary.path}
      verb="Edited"
    />
  );
}

function CompactMessageSummary({
  duration,
  isError,
  label,
  preview,
}: {
  duration: string | null;
  isError: boolean;
  label: string;
  preview: string | null;
}) {
  const mutedTone = compactSummaryTone();
  return (
    <div className="flex max-w-[85%] flex-col items-start gap-1.5">
      <div
        className={cn(
          "min-w-0 rounded-2xl border px-3 py-2 text-sm leading-relaxed shadow-sm",
          isError
            ? "border-destructive/25 bg-destructive/10 text-destructive"
            : "border-primary/15 bg-primary/6 text-foreground",
        )}
        data-testid="transcript-tool-message-preview"
      >
        <p className="whitespace-pre-wrap wrap-break-word">
          {preview || "Message content unavailable."}
        </p>
      </div>
      <div className="inline-flex max-w-full items-center gap-1.5 px-1">
        <Send className={cn("h-3.5 w-3.5 shrink-0", mutedTone)} />
        <span className={cn("truncate text-xs font-medium", mutedTone)}>
          {label}
        </span>
        {duration ? (
          <span className={cn("shrink-0 text-xs", mutedTone)}>{duration}</span>
        ) : null}
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 transition-transform group-open:rotate-180",
            mutedTone,
          )}
        />
      </div>
    </div>
  );
}

function ViewImageToolPreview({
  src,
  title,
}: {
  src: string;
  title: string | null;
}) {
  const [lightboxOpen, setLightboxOpen] = React.useState(false);
  const [imageFailed, setImageFailed] = React.useState(false);
  const resolvedSrc = React.useMemo(() => resolveImageSrc(src), [src]);
  const alt = title ?? "Viewed image";

  if (imageFailed) {
    return null;
  }

  return (
    <>
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: opens lightbox on click */}
      <img
        alt={alt}
        className="ml-1.5 block max-h-64 max-w-[min(24rem,calc(100%-0.375rem))] cursor-pointer rounded-lg object-contain"
        decoding="async"
        loading="lazy"
        onClick={() => setLightboxOpen(true)}
        onError={() => setImageFailed(true)}
        src={resolvedSrc}
        title={title ?? undefined}
      />
      <ImageLightbox
        alt={alt}
        onOpenChange={setLightboxOpen}
        open={lightboxOpen}
        src={resolvedSrc}
      />
    </>
  );
}

function ImageLightbox({
  alt,
  onOpenChange,
  open,
  src,
}: {
  alt: string;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  src: string;
}) {
  return (
    <DialogPrimitive.Root open={open} onOpenChange={onOpenChange}>
      <DialogPrimitive.Portal>
        <DialogPrimitive.Overlay className="fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:fade-in-0" />
        <DialogPrimitive.Content
          className="fixed inset-0 z-50 flex items-center justify-center p-8"
          onInteractOutside={(event) => event.preventDefault()}
          onPointerDownOutside={(event) => event.preventDefault()}
        >
          <DialogPrimitive.Title className="sr-only">
            {alt}
          </DialogPrimitive.Title>
          <DialogPrimitive.Description className="sr-only">
            Full-size image preview. Press Escape or click outside the image to
            close.
          </DialogPrimitive.Description>
          <DialogPrimitive.Close
            aria-label="Close lightbox"
            className="absolute inset-0 cursor-default"
          />
          <img
            alt={alt}
            className="relative max-h-[90vh] max-w-[90vw] rounded-lg object-contain"
            src={src}
          />
          <DialogPrimitive.Close className="absolute right-4 top-4 rounded-full bg-black/50 p-2 text-white/80 transition-colors hover:bg-black/70 hover:text-white focus:outline-hidden focus:ring-2 focus:ring-white/30">
            <svg
              aria-hidden="true"
              fill="none"
              height="20"
              viewBox="0 0 24 24"
              width="20"
              xmlns="http://www.w3.org/2000/svg"
            >
              <path
                d="M18 6L6 18M6 6l12 12"
                stroke="currentColor"
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth="2"
              />
            </svg>
          </DialogPrimitive.Close>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

function ToolDetailBlocks({
  args,
  description,
  fileEditDiff,
  hasArgs,
  hasResult,
  imagePreview,
  isError,
  result,
}: {
  args: Record<string, unknown>;
  description?: string;
  fileEditDiff: FileEditDiff | null;
  hasArgs: boolean;
  hasResult: boolean;
  imagePreview: { src: string | null; title: string | null } | null;
  isError: boolean;
  result: string;
}) {
  const showFileEditDiff =
    fileEditDiff && hasLineDiff(fileEditDiff) && !isError;
  const showParameters = hasArgs && !showFileEditDiff;

  return (
    <div className="space-y-4 py-2 text-popover-foreground outline-hidden">
      {description ? (
        <p className="max-w-2xl text-xs leading-5 text-muted-foreground">
          {description}
        </p>
      ) : null}
      {imagePreview?.src ? (
        <ViewImageToolPreview
          src={imagePreview.src}
          title={imagePreview.title}
        />
      ) : null}
      {showParameters ? (
        <ToolCodeBlock
          label="Parameters"
          tone="muted"
          value={JSON.stringify(args, null, 2)}
        />
      ) : null}
      {hasResult ? (
        showFileEditDiff ? (
          <FileEditDiffBlock diff={fileEditDiff} />
        ) : (
          <ToolCodeBlock
            label={isError ? "Error" : "Result"}
            tone={isError ? "error" : "muted"}
            value={result}
          />
        )
      ) : null}
      {!showParameters && !hasResult ? (
        <p className="text-sm text-muted-foreground/80">
          Waiting for tool details.
        </p>
      ) : null}
    </div>
  );
}

function hasLineDiff(diff: FileEditDiff) {
  return diff.lines.some(
    (line) => line.kind === "add" || line.kind === "remove",
  );
}

function FileEditDiffBlock({ diff }: { diff: FileEditDiff }) {
  return (
    <div className="flex max-h-64 flex-col overflow-hidden rounded-md border border-border/50 bg-muted/35 text-xs leading-5 text-foreground">
      <pre className="min-h-0 flex-1 overflow-auto py-2 font-mono">
        {diff.lines
          .filter((line) => line.kind !== "meta")
          .map((line, index) => (
            <FileEditDiffLineView
              // biome-ignore lint/suspicious/noArrayIndexKey: diff lines are positional
              key={index}
              line={line}
            />
          ))}
      </pre>
      <div
        className="truncate border-t border-border/50 px-3 py-1.5 text-xs font-normal text-muted-foreground/70"
        title={diff.path}
      >
        {diff.path}
      </div>
    </div>
  );
}

function FileEditDiffLineView({ line }: { line: FileEditDiffLine }) {
  return (
    <span
      className={cn(
        "block min-w-full whitespace-pre-wrap wrap-break-word px-3",
        line.kind === "add" &&
          "border-l-2 border-green-500/50 bg-green-500/12 text-foreground dark:bg-green-500/10",
        line.kind === "remove" &&
          "border-l-2 border-red-500/50 bg-red-500/12 text-foreground dark:bg-red-500/10",
        line.kind === "meta" && "text-muted-foreground/70",
      )}
    >
      {line.text || " "}
    </span>
  );
}

function ToolCodeBlock({
  label,
  tone,
  value,
}: {
  label: string;
  tone: "muted" | "error";
  value: string;
}) {
  return (
    <div className="space-y-2 overflow-hidden">
      <h4 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </h4>
      <pre
        className={cn(
          "max-h-64 overflow-auto whitespace-pre-wrap wrap-break-word rounded-md px-3 py-2 font-mono text-xs leading-5",
          tone === "error"
            ? "bg-destructive/10 text-destructive"
            : "bg-muted/50 text-foreground",
        )}
      >
        {formatCodeValue(value)}
      </pre>
    </div>
  );
}
