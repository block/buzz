import * as React from "react";
import { FileDiff, Maximize2 } from "lucide-react";

import { getDiffTitleBadge } from "@/features/messages/lib/parseDiff";
import { isSafeUrl } from "@/shared/lib/url";
import { Button } from "@/shared/ui/button";
import { useSmoothCorners } from "@/shared/ui/smoothCorners";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { DiffViewer } from "./DiffViewer";

type DiffMessageProps = {
  content: string;
  repoUrl?: string;
  filePath?: string;
  commitSha?: string;
  description?: string;
  truncated?: boolean;
  onExpand?: () => void;
};

function getHostname(url: string): string {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

export default function DiffMessage({
  content,
  repoUrl,
  filePath,
  commitSha,
  description,
  truncated,
  onExpand,
}: DiffMessageProps) {
  const diffCardRef = React.useRef<HTMLDivElement | null>(null);
  useSmoothCorners(diffCardRef);

  const safeRepoUrl = isSafeUrl(repoUrl) ? repoUrl : undefined;

  const titleBadge = React.useMemo(
    () => getDiffTitleBadge(content, filePath),
    [content, filePath],
  );

  const commitUrl =
    safeRepoUrl && commitSha ? `${safeRepoUrl}/commit/${commitSha}` : undefined;

  const shortSha = commitSha ? commitSha.slice(0, 7) : undefined;

  return (
    <div
      ref={diffCardRef}
      className="overflow-hidden rounded-2xl border border-border/70 bg-card/60 text-sm"
    >
      <div className="flex items-center gap-2 px-3 py-2 border-b border-border/50 bg-muted/40">
        <FileDiff className="h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 truncate font-mono text-xs text-foreground/80">
          {filePath ?? "diff"}
        </span>
        {titleBadge && (
          <span className="shrink-0 rounded-md border border-border/60 px-1.5 py-0.5 text-2xs uppercase tracking-[0.14em] text-muted-foreground">
            {titleBadge}
          </span>
        )}
        <div className="ml-auto flex items-center gap-2">
          {shortSha && (
            <span className="text-xs text-muted-foreground font-mono">
              {commitUrl ? (
                <a
                  className="hover:underline"
                  href={commitUrl}
                  rel="noreferrer noopener"
                  target="_blank"
                >
                  {shortSha}
                </a>
              ) : (
                shortSha
              )}
            </span>
          )}
          {safeRepoUrl && !commitUrl && (
            <span className="text-xs text-muted-foreground">
              <a
                className="hover:underline"
                href={safeRepoUrl}
                rel="noreferrer noopener"
                target="_blank"
              >
                {getHostname(safeRepoUrl)}
              </a>
            </span>
          )}
          {onExpand && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  aria-label="Expand diff"
                  className="h-6 w-6 p-0 text-muted-foreground hover:text-foreground"
                  onClick={onExpand}
                  size="sm"
                  type="button"
                  variant="ghost"
                >
                  <Maximize2 className="h-4 w-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Expand diff</TooltipContent>
            </Tooltip>
          )}
        </div>
      </div>

      {description && (
        <div className="px-3 py-1.5 text-xs text-muted-foreground border-b border-border/40 bg-muted/20">
          {description}
        </div>
      )}

      {/* Diff content — max 400px height, scrollable */}
      <div className="max-h-[400px] overflow-auto text-xs">
        <DiffViewer
          className="p-3"
          content={content}
          fallbackFilePath={filePath}
          viewType="unified"
        />
      </div>

      {/* Truncation warning */}
      {truncated && (
        <div className="px-3 py-2 border-t border-border/50 bg-amber-500/10 text-xs text-warning">
          Diff truncated.{" "}
          {safeRepoUrl && commitUrl ? (
            <a
              className="underline hover:no-underline"
              href={commitUrl}
              rel="noreferrer noopener"
              target="_blank"
            >
              View full diff on {getHostname(safeRepoUrl)}
            </a>
          ) : (
            "View the full diff at the source repository."
          )}
        </div>
      )}
    </div>
  );
}
