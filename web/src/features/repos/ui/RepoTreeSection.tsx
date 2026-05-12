import { File, Folder } from "lucide-react";
import type { TreeEntry } from "../git-client";

function TreeRow({ entry }: { entry: TreeEntry }) {
  const isDir = entry.type === "tree";
  return (
    <div className="flex items-center gap-2 border-b border-border px-3 py-2 text-sm last:border-b-0">
      {isDir ? (
        <Folder className="h-4 w-4 shrink-0 text-blue-400" />
      ) : (
        <File className="h-4 w-4 shrink-0 text-muted-foreground" />
      )}
      <span className={isDir ? "font-medium" : ""}>{entry.name}</span>
    </div>
  );
}

export function RepoTreeSection({
  entries,
  isLoading,
}: {
  entries: TreeEntry[] | undefined;
  isLoading: boolean;
}) {
  if (isLoading) {
    return (
      <div className="mt-8">
        <div className="rounded-lg border border-border">
          {["sk-1", "sk-2", "sk-3", "sk-4", "sk-5"].map((key) => (
            <div
              key={key}
              className="flex items-center gap-2 border-b border-border px-3 py-2 last:border-b-0"
            >
              <div className="h-4 w-4 animate-pulse rounded bg-muted" />
              <div className="h-4 w-32 animate-pulse rounded bg-muted" />
            </div>
          ))}
        </div>
      </div>
    );
  }

  if (!entries || entries.length === 0) return null;

  return (
    <div className="mt-8">
      <div className="overflow-hidden rounded-lg border border-border">
        {entries.map((entry) => (
          <TreeRow key={entry.name} entry={entry} />
        ))}
      </div>
    </div>
  );
}
