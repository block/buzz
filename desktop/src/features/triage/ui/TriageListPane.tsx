import type { TriageSuggestion } from "@/features/triage/api";
import { TriageRow } from "@/features/triage/ui/TriageRow";
import { Skeleton } from "@/shared/ui/skeleton";

type TriageListPaneProps = {
  emptyMessage: string;
  isLoading: boolean;
  onSelect: (eventId: string) => void;
  selectedEventId: string | null;
  suggestions: readonly TriageSuggestion[];
};

function ListSkeleton() {
  return (
    <div className="space-y-3 p-3">
      {["one", "two", "three", "four"].map((row) => (
        <div className="space-y-1.5" key={row}>
          <Skeleton className="h-4 w-32" />
          <Skeleton className="h-4 w-full" />
          <Skeleton className="h-3 w-2/3" />
        </div>
      ))}
    </div>
  );
}

export function TriageListPane({
  emptyMessage,
  isLoading,
  onSelect,
  selectedEventId,
  suggestions,
}: TriageListPaneProps) {
  if (isLoading) {
    return <ListSkeleton />;
  }

  if (suggestions.length === 0) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <p className="text-center text-sm text-muted-foreground">
          {emptyMessage}
        </p>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-col overflow-y-auto">
      {suggestions.map((suggestion) => (
        <TriageRow
          isSelected={suggestion.eventId === selectedEventId}
          key={suggestion.eventId}
          onSelect={onSelect}
          suggestion={suggestion}
        />
      ))}
    </div>
  );
}
