import { cn } from "@/shared/lib/cn";
import type { FileEditDiff } from "../agentSessionFileEditDiff";
import type { FileReadContent } from "../agentSessionFileRead";
import { FileContentBlock } from "../FileContentBlock";
import { FileEditDiffBlock, hasFileEditLineDiff } from "../FileEditDiffView";
import { formatCodeValue } from "../agentSessionUtils";
import { ShellCommandBlock } from "./ShellCommandBlock";
import { ViewImageToolPreview } from "./ViewImageToolPreview";

export function ToolDetailBlocks({
  args,
  description,
  fileEditDiff,
  fileReadContent,
  hasArgs,
  hasResult,
  imagePreview,
  isError,
  result,
  shellCommand,
}: {
  args: Record<string, unknown>;
  description?: string;
  fileEditDiff: FileEditDiff | null;
  fileReadContent: FileReadContent | null;
  hasArgs: boolean;
  hasResult: boolean;
  imagePreview: { src: string | null; title: string | null } | null;
  isError: boolean;
  result: string;
  shellCommand: string | null;
}) {
  const showFileEditDiff =
    fileEditDiff && hasFileEditLineDiff(fileEditDiff) && !isError;
  const showFileReadContent = fileReadContent != null && !isError;
  const showFileContent = showFileEditDiff || showFileReadContent;
  const showShellCommand = shellCommand != null && !showFileContent;
  const showParameters = hasArgs && !showFileContent;

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
      {showShellCommand ? (
        <ShellCommandBlock command={shellCommand} result={result} />
      ) : showParameters ? (
        <ToolCodeBlock
          label="Parameters"
          tone="muted"
          value={JSON.stringify(args, null, 2)}
        />
      ) : null}
      {!showShellCommand && hasResult ? (
        showFileEditDiff ? (
          <FileEditDiffBlock diff={fileEditDiff} />
        ) : showFileReadContent ? (
          <FileContentBlock
            footerText={fileReadContent.footerText}
            footerTitle={fileReadContent.footerTitle}
            lines={fileReadContent.lines}
            path={fileReadContent.path}
          />
        ) : (
          <ToolCodeBlock
            label={isError ? "Error" : "Result"}
            tone={isError ? "error" : "muted"}
            value={result}
          />
        )
      ) : null}
      {!showShellCommand && !showParameters && !hasResult ? (
        <p className="text-sm text-muted-foreground/80">
          Waiting for tool details.
        </p>
      ) : null}
    </div>
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
