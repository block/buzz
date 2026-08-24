import { LoaderCircle } from "lucide-react";

import { UserAvatar } from "@/shared/ui/UserAvatar";

export function WorkflowRichTriggerDescription({
  avatarUrl,
  description,
  label,
  loading,
}: {
  avatarUrl?: string | null;
  description: string;
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

  const labelIndex = label ? description.lastIndexOf(label) : -1;
  if (!label || labelIndex < 0) return description;

  const prefix = description.slice(0, labelIndex).trimEnd();
  const suffix = description.slice(labelIndex + label.length).trimStart();
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      {prefix ? <span className="shrink-0">{prefix}</span> : null}
      <UserAvatar
        avatarUrl={avatarUrl ?? null}
        className="h-4 w-4"
        displayName={label}
        fallbackDelayMs={0}
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
