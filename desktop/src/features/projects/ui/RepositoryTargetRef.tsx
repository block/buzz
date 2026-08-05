import { GitBranch, GitCommit } from "lucide-react";

export function RepositoryTargetRef({ value }: { value: string }) {
  const isCommit = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/i.test(value);
  const Icon = isCommit ? GitCommit : GitBranch;
  const label = isCommit ? value.slice(0, 7) : value;
  return (
    <span
      className="flex h-7 min-w-0 max-w-64 items-center gap-1.5 rounded-md border border-input bg-background px-3 font-mono text-sm font-medium"
      title={value}
    >
      <Icon className="h-4 w-4 shrink-0 text-muted-foreground" />
      <span className="truncate">{label}</span>
    </span>
  );
}
