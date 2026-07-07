import { Plus } from "lucide-react";

import { Button } from "@/shared/ui/button";

type ChatListSectionHeaderProps = {
  actionLabel?: string;
  label: string;
  onAction?: () => void;
};

export function ChatListSectionHeader({
  actionLabel,
  label,
  onAction,
}: ChatListSectionHeaderProps) {
  return (
    <div className="group/section flex h-8 items-center gap-2 rounded-md px-2 text-xs font-medium text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground">
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {onAction ? (
        <Button
          aria-label={actionLabel ?? label}
          className="h-6 w-6 text-muted-foreground opacity-0 transition-opacity hover:text-foreground focus-visible:opacity-100 group-hover/section:opacity-100"
          onClick={onAction}
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          <Plus className="h-3.5 w-3.5" />
        </Button>
      ) : null}
    </div>
  );
}
