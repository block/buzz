import { LoaderCircle } from "lucide-react";

import { UserAvatar } from "@/shared/ui/UserAvatar";
import { splitWorkflowAuthorDescription } from "./workflowTriggerDescription";

export function WorkflowRichTriggerDescription({
  avatarUrl,
  description,
  isAgent,
  label,
  loading,
}: {
  avatarUrl?: string | null;
  description: string;
  isAgent?: boolean;
  label?: string | null;
  loading?: boolean;
}) {
  if (loading) {
    return (
      <span className="flex min-w-0 items-center gap-1.5">
        <span className="truncate">{description}</span>
        <LoaderCircle
          aria-label="Loading author"
          className="h-3.5 w-3.5 shrink-0 animate-spin"
          role="status"
        />
      </span>
    );
  }

  const segments = label
    ? splitWorkflowAuthorDescription(description, label)
    : null;
  if (!label || !segments) return description;

  const { prefix, suffix } = segments;
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      {prefix ? <span className="shrink-0">{prefix}</span> : null}
      <UserAvatar
        avatarUrl={avatarUrl ?? null}
        className="h-4 w-4"
        displayName={label}
        fallbackDelayMs={0}
        shape={isAgent ? "squircle" : "circle"}
        size="xs"
        testId="workflow-trigger-author-avatar"
      />
      <span className="min-w-0 truncate">
        {label}
        {suffix ? ` ${suffix}` : ""}
      </span>
    </span>
  );
}
