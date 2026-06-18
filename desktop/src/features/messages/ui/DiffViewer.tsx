import { Diff, Hunk, type ViewType } from "react-diff-view";
import "react-diff-view/style/index.css";
import { useMemo } from "react";

import {
  countDiffFileChanges,
  getDiffFileLabel,
  normalizeDiffType,
  parseUnifiedDiff,
} from "@/features/messages/lib/parseDiff";
import { cn } from "@/shared/lib/cn";
import "./DiffViewer.css";

type DiffViewerProps = {
  content: string;
  fallbackFilePath?: string;
  viewType?: ViewType;
  className?: string;
};

const DIFF_TYPE_LABELS = {
  add: "New file",
  copy: "Copied",
  delete: "Deleted",
  modify: "Modified",
  rename: "Renamed",
} as const;

function FileChangeBadge({
  tone,
  value,
}: {
  tone: "positive" | "negative";
  value: string;
}) {
  return (
    <span
      className={cn(
        "rounded-md px-1.5 py-0.5 font-mono text-2xs font-semibold",
        tone === "positive"
          ? "bg-emerald-500/10 text-status-added"
          : "bg-rose-500/10 text-status-deleted",
      )}
    >
      {value}
    </span>
  );
}

export function DiffViewer({
  content,
  fallbackFilePath,
  viewType = "unified",
  className,
}: DiffViewerProps) {
  const { files, parseError } = useMemo(
    () => parseUnifiedDiff(content),
    [content],
  );

  if (parseError) {
    return (
      <pre className="p-3 whitespace-pre-wrap font-mono text-xs text-muted-foreground">
        {content}
      </pre>
    );
  }

  if (!files.length) {
    return (
      <div className="p-3 text-xs italic text-muted-foreground">
        No diff content
      </div>
    );
  }

  return (
    <div className={cn("buzz-diff-theme", className)}>
      <div className="space-y-3">
        {files.map((file) => {
          const label = getDiffFileLabel(file, fallbackFilePath);
          const showFileHeader =
            files.length > 1 || !fallbackFilePath || label !== fallbackFilePath;
          const { additions, deletions } = countDiffFileChanges(file);
          const diffType = normalizeDiffType(file.type);
          const fileKey = [
            file.oldPath || "",
            file.newPath || "",
            file.oldRevision || "",
            file.newRevision || "",
          ].join(":");

          return (
            <section
              className="overflow-hidden rounded-xl border border-border/60 bg-background/40"
              key={fileKey || label}
            >
              {showFileHeader ? (
                <div className="flex items-center gap-2 border-b border-border/60 bg-muted/35 px-3 py-2">
                  <span className="truncate font-mono text-2xs text-foreground/85">
                    {label}
                  </span>
                  <span className="rounded-md border border-border/60 px-1.5 py-0.5 text-2xs uppercase tracking-[0.14em] text-muted-foreground">
                    {DIFF_TYPE_LABELS[diffType]}
                  </span>
                  <div className="ml-auto flex items-center gap-1.5">
                    {additions > 0 ? (
                      <FileChangeBadge
                        tone="positive"
                        value={`+${additions}`}
                      />
                    ) : null}
                    {deletions > 0 ? (
                      <FileChangeBadge
                        tone="negative"
                        value={`-${deletions}`}
                      />
                    ) : null}
                  </div>
                </div>
              ) : null}

              {file.hunks.length > 0 ? (
                <Diff
                  className={cn(
                    "buzz-diff-table",
                    viewType === "split" ? "min-w-[780px]" : "w-full",
                  )}
                  codeClassName="buzz-diff-code"
                  diffType={diffType}
                  gutterClassName="buzz-diff-gutter"
                  hunks={file.hunks}
                  lineClassName="buzz-diff-line"
                  viewType={viewType}
                >
                  {(hunks) =>
                    hunks.map((hunk) => (
                      <Hunk
                        hunk={hunk}
                        key={`${fileKey}:${hunk.oldStart}:${hunk.newStart}:${hunk.content}`}
                      />
                    ))
                  }
                </Diff>
              ) : (
                <div className="px-3 py-3 text-xs text-muted-foreground">
                  No textual hunks in this diff.
                </div>
              )}
            </section>
          );
        })}
      </div>
    </div>
  );
}
