import { AlertCircle } from "lucide-react";

import type { ProjectWorkItemSection } from "@/features/projects/projectWorkItems";
import { useTranslation } from "@/i18n";
import { Button } from "@/shared/ui/button";

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
  const { t } = useTranslation();
  if (!error && failedSections.length === 0) return null;

  const sectionLabels: Record<ProjectWorkItemSection, string> = {
    assignments: t("projects.work-items-notice.section-assignments"),
    comments: t("projects.work-items-notice.section-comments"),
    "pull-request-updates": t(
      "projects.work-items-notice.section-review-updates",
    ),
    statuses: t("projects.work-items-notice.section-statuses"),
  };
  const displaySubject =
    subject === "pull requests"
      ? t("projects.work-items-notice.subject-reviews")
      : subject === "issues"
        ? t("projects.work-items-notice.subject-tasks")
        : subject;
  const detailSubject =
    subject === "pull requests"
      ? t("projects.work-items-notice.subject-review")
      : subject === "issues"
        ? t("projects.work-items-notice.subject-task")
        : subject;
  const title = error
    ? t("projects.work-items-notice.could-not-load", {
        subject: displaySubject,
      })
    : t("projects.work-items-notice.partial-details", {
        subject: detailSubject,
      });
  const description = error
    ? error instanceof Error
      ? error.message
      : t("projects.work-items-notice.relay-failed")
    : t("projects.work-items-notice.missing-sections", {
        sections: failedSections
          .map((section) => sectionLabels[section])
          .join(", "),
      });

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
