import { BookOpen } from "lucide-react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { ReadmeResult } from "../git-client";

export function RepoReadmeSection({
  readme,
  isLoading,
}: {
  readme: ReadmeResult | null | undefined;
  isLoading: boolean;
}) {
  if (isLoading) {
    return (
      <div className="mt-8">
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
          <BookOpen className="h-4 w-4" />
          README
        </h2>
        <div className="space-y-2 rounded-lg border border-border p-4">
          <div className="h-4 w-3/4 animate-pulse rounded bg-muted" />
          <div className="h-4 w-1/2 animate-pulse rounded bg-muted" />
          <div className="h-4 w-5/6 animate-pulse rounded bg-muted" />
        </div>
      </div>
    );
  }

  if (!readme) return null;

  return (
    <div className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold">
        <BookOpen className="h-4 w-4" />
        {readme.filename}
      </h2>
      <div className="prose prose-sm dark:prose-invert max-w-none rounded-lg border border-border p-4">
        <Markdown remarkPlugins={[remarkGfm]}>{readme.content}</Markdown>
      </div>
    </div>
  );
}
