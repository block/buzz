import { cn } from "@/shared/lib/cn";
import type {
  FileEditDiff,
  FileEditDiffLine,
} from "./agentSessionFileEditDiff";

export function hasFileEditLineDiff(diff: FileEditDiff) {
  return diff.lines.some(
    (line) => line.kind === "add" || line.kind === "remove",
  );
}

export function FileEditDiffBlock({ diff }: { diff: FileEditDiff }) {
  return (
    <div className="flex max-h-64 flex-col overflow-hidden rounded-md border border-border/50 bg-muted/35 text-xs leading-5 text-foreground">
      <pre className="min-h-0 flex-1 overflow-auto py-2 font-mono">
        <FileEditDiffLines diff={diff} />
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

function FileEditDiffLines({ diff }: { diff: FileEditDiff }) {
  return diff.lines
    .filter((line) => line.kind !== "meta")
    .map((line, index) => (
      <FileEditDiffLineView
        // biome-ignore lint/suspicious/noArrayIndexKey: diff lines are positional
        key={index}
        line={line}
      />
    ));
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
