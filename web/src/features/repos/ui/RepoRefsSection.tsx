import { GitBranch, Hash, Tag } from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import type { RepoRefs } from "../use-repo-refs";

export function RepoRefsSection({
  refs,
  isLoading,
}: {
  refs: RepoRefs | undefined;
  isLoading: boolean;
}) {
  if (isLoading) return null;

  const hasRefs = refs && (refs.branches.length > 0 || refs.tags.length > 0);

  return (
    <div className="mt-6">
      {hasRefs ? (
        <div className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground">
          {refs.head && (
            <>
              <div className="flex items-center gap-1.5">
                <Badge variant="secondary">
                  <GitBranch className="mr-1 h-4 w-4" />
                  {refs.head.ref}
                </Badge>
                {refs.head.sha && (
                  <Badge variant="outline" className="font-mono text-xs">
                    <Hash className="mr-0.5 h-4 w-4" />
                    {refs.head.sha.slice(0, 7)}
                  </Badge>
                )}
              </div>
              <span className="text-muted-foreground/60">&middot;</span>
            </>
          )}
          <span className="flex items-center gap-1">
            <GitBranch className="h-4 w-4" />
            {refs.branches.length}{" "}
            {refs.branches.length === 1 ? "branch" : "branches"}
          </span>
          <span className="text-muted-foreground/60">&middot;</span>
          <span className="flex items-center gap-1">
            <Tag className="h-4 w-4" />
            {refs.tags.length} {refs.tags.length === 1 ? "tag" : "tags"}
          </span>
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">No commits yet</p>
      )}
    </div>
  );
}
