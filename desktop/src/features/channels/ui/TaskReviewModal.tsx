import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ClipboardCheck } from "lucide-react";
import type { ChannelBackedTask } from "../lib/channelBackedTask";
import { useTaskBranchStatus } from "../useTaskBranchStatus";
import { invokeTauri } from "@/shared/api/tauri";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogDescription,
  DialogTrigger,
} from "@/shared/ui/dialog";

type Review = {
  name: string;
  description?: string;
  invocation_ok?: boolean;
  finding_count?: number;
  log?: string;
  verdict?: {
    overall_correctness?: string;
    overall_explanation?: string;
    findings?: { title: string; body: string }[];
  };
};
type Run = {
  head?: string;
  started_at?: string;
  pr_metadata?: { title?: string; body?: string };
  reviews?: Review[];
};
type Reviews = { runs: Run[]; unattributed: number; skipped: number };

export function TaskReviewModal({
  branch,
}: {
  branch: NonNullable<ChannelBackedTask["branch"]>;
}) {
  const [open, setOpen] = useState(false);
  const { pr } = useTaskBranchStatus(branch);
  const query = useQuery({
    queryKey: ["task-reviews", branch.repository, branch.name, pr?.url],
    enabled: open,
    queryFn: () =>
      invokeTauri<Reviews>("get_task_reviews", {
        repository: branch.repository,
        branch: branch.name,
        prUrl: pr?.url ?? null,
      }),
    staleTime: 0,
    retry: false,
  });
  const latest = new Map<string, { run: Run; review: Review }>();
  for (const run of query.data?.runs ?? []) {
    for (const review of run.reviews ?? []) {
      if (!latest.has(review.name)) latest.set(review.name, { run, review });
    }
  }
  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogTrigger asChild>
        <button
          type="button"
          className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        >
          <ClipboardCheck aria-hidden="true" className="size-3.5" />
          Reviews
        </button>
      </DialogTrigger>
      <DialogContent className="max-h-[80vh] overflow-y-auto">
        <DialogTitle>Local reviews</DialogTitle>
        <DialogDescription>
          Latest recorded run per intent for {branch.name}. Matching commits do
          not verify the base range or whether findings were addressed.
        </DialogDescription>
        {query.isPending && <p role="status">Loading reviews…</p>}
        {query.isError && <p role="alert">{String(query.error)}</p>}
        {query.data && latest.size === 0 && (
          <p>No attributable local runs found for this branch.</p>
        )}
        {[...latest].map(([name, { run, review }]) => {
          const freshness =
            name === "pr-template"
              ? typeof run.pr_metadata?.title !== "string" ||
                typeof run.pr_metadata?.body !== "string" ||
                pr?.body === undefined
                ? "Metadata freshness unknown"
                : run.pr_metadata.title === pr.title &&
                    run.pr_metadata.body === pr.body
                  ? "Same PR metadata"
                  : "PR metadata changed"
              : !run.head || !pr?.headRefOid
                ? "Freshness unknown"
                : run.head !== pr.headRefOid
                  ? "Different commit"
                  : "Same commit";
          return (
            <details key={name} className="rounded-lg border p-3 text-sm">
              <summary className="cursor-pointer">
                <span className="font-medium">{name}</span>
                <span className="ml-2 text-muted-foreground">
                  {review.invocation_ok === false
                    ? "Run failed"
                    : review.finding_count !== undefined ||
                        review.verdict?.findings
                      ? `${review.finding_count ?? review.verdict?.findings?.length} findings`
                      : "Result unknown"}{" "}
                  · {freshness}
                </span>
              </summary>
              <div className="mt-3 space-y-2">
                <p className="text-xs text-muted-foreground">
                  {run.started_at ?? "Date unknown"} · Commit{" "}
                  {run.head?.slice(0, 12) ?? "unknown"}
                </p>
                <p>{review.description}</p>
                <p>
                  {review.verdict?.overall_correctness}{" "}
                  {review.verdict?.overall_explanation}
                </p>
                {review.verdict?.findings?.map((finding) => (
                  <div
                    key={`${finding.title}-${finding.body}`}
                    className="whitespace-pre-wrap break-words border-t pt-2"
                  >
                    <p className="font-medium">{finding.title}</p>
                    <p>{finding.body}</p>
                  </div>
                ))}
                {review.log && (
                  <p className="break-all text-xs text-muted-foreground">
                    Local log: {review.log}
                  </p>
                )}
              </div>
            </details>
          );
        })}
        {!!query.data?.unattributed && (
          <p className="text-xs text-muted-foreground">
            {query.data.unattributed} other or unattributable repository runs
            excluded.
          </p>
        )}
        {!!query.data?.skipped && (
          <p className="text-xs text-muted-foreground">
            {query.data.skipped} unavailable or invalid cache entries skipped.
          </p>
        )}
        <button
          type="button"
          disabled={query.isFetching}
          onClick={() => void query.refetch()}
          className="justify-self-start text-sm underline"
        >
          Refresh
        </button>
      </DialogContent>
    </Dialog>
  );
}
