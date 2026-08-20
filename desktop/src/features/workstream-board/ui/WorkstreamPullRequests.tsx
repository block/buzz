import { ExternalLink, GitPullRequest } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { parseEntityLink } from "@/shared/lib/entityLink";
import { useOpenEntityLink } from "@/shared/ui/markdown/entityLinks";
import type {
  WorkstreamPullRequestDisplayState,
  WorkstreamPullRequestReference,
} from "@/features/workstream-board/lib/workstreamPullRequestStatus";

function lifecycleLabel(
  state: Extract<WorkstreamPullRequestDisplayState, { status: "live" }>,
) {
  return state.lifecycle[0].toUpperCase() + state.lifecycle.slice(1);
}

function reviewLabel(
  state: Extract<WorkstreamPullRequestDisplayState, { status: "live" }>,
) {
  switch (state.reviewState) {
    case "approved":
      return "Approved";
    case "changes-requested":
      return "Changes requested";
    case "review-requested":
      return "Review requested";
    default:
      return "No reviews";
  }
}

function referenceLabel(reference: WorkstreamPullRequestReference) {
  if (reference.kind === "github") {
    return `${reference.owner}/${reference.repository} #${reference.number}`;
  }
  if (reference.kind === "buzz") {
    return `${reference.repository} · ${reference.eventId.slice(0, 8)}`;
  }
  return reference.rawUrl;
}

export function WorkstreamPullRequests({
  references,
  states,
}: {
  references: readonly WorkstreamPullRequestReference[];
  states: ReadonlyMap<string, WorkstreamPullRequestDisplayState>;
}) {
  const openEntityLink = useOpenEntityLink();

  return (
    <div
      className="pointer-events-auto mt-4 space-y-1.5"
      data-testid="workstream-pull-requests"
    >
      <p className="text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
        Pull requests
      </p>
      {references.map((reference) => {
        const state = states.get(reference.identity);
        const label = referenceLabel(reference);
        const parsedBuzzLink =
          reference.kind === "buzz" ? parseEntityLink(reference.href) : null;
        const open = () => {
          if (parsedBuzzLink?.ok) {
            // Keep Buzz PRs in the existing canonical entity navigation path.
            openEntityLink(parsedBuzzLink.value);
            return;
          }
          void openUrl(reference.href);
        };

        return (
          <button
            className="flex w-full items-center gap-2 rounded-md border border-border/50 bg-background/60 px-2 py-1.5 text-left text-2xs transition-colors hover:bg-background focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
            data-testid="workstream-pull-request"
            key={reference.identity}
            onClick={open}
            type="button"
          >
            <GitPullRequest className="h-3.5 w-3.5 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate" title={reference.rawUrl}>
              {state?.status === "live" && state.title
                ? `${label} · ${state.title}`
                : label}
            </span>
            {state?.status === "loading" ? (
              <span className="shrink-0 text-muted-foreground">Loading…</span>
            ) : state?.status === "live" ? (
              <span className="shrink-0 text-muted-foreground">
                {lifecycleLabel(state)} · {reviewLabel(state)}
              </span>
            ) : state?.status === "error" ? (
              <span className="shrink-0 text-red-400">Unavailable</span>
            ) : state?.status === "unknown" ? (
              <span className="shrink-0 text-muted-foreground">Unknown</span>
            ) : state?.status === "unsupported" ? (
              <span className="shrink-0 text-muted-foreground">
                Unsupported
              </span>
            ) : null}
            <ExternalLink className="h-3 w-3 shrink-0 text-muted-foreground" />
          </button>
        );
      })}
    </div>
  );
}
