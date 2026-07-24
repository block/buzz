import { AlertCircle } from "lucide-react";

import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import { Button } from "@/shared/ui/button";

const SECTION_LABELS: Record<ProjectWorkItemSection, string> = {
  comments: "comments",
  "pull-request-updates": "pull request updates",
  statuses: "statuses",
};

type ProjectsWorkItemsLoadNoticeProps = {
  error: unknown;
  failedSections: ProjectWorkItemSection[];
  isRetrying: boolean;
  onRetry: () => void;
  subject: "issues" | "project activity" | "pull requests";
};

/** Displays full and partial aggregate work-item failures with a retry action. */
export function ProjectsWorkItemsLoadNotice({
  error,
  failedSections,
  isRetrying,
  onRetry,
  subject,
}: ProjectsWorkItemsLoadNoticeProps) {
  if (!error && failedSections.length === 0) return null;

  const detailSubject =
    subject === "pull requests"
      ? "pull request"
      : subject === "issues"
        ? "issue"
        : subject;
  const title = error
    ? `Could not load ${subject}.`
    : `Some ${detailSubject} details could not be loaded.`;
  const description = error
    ? error instanceof Error
      ? error.message
      : "The relay request failed."
    : `Missing ${failedSections
        .map((section) => SECTION_LABELS[section])
        .join(", ")}. The available results are shown below.`;

  return (
    <div
      className="flex items-start gap-3 border border-destructive/30 bg-destructive/5 px-4 py-3 text-sm"
      role="alert"
    >
      <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-foreground">{title}</p>
        <p className="mt-0.5 text-muted-foreground">{description}</p>
      </div>
      <Button
        disabled={isRetrying}
        onClick={onRetry}
        size="sm"
        variant="outline"
      >
        {isRetrying ? "Retrying..." : "Retry"}
      </Button>
    </div>
  );
}
