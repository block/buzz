import { Button } from "@/shared/ui/button";

export type TriageTab = "important" | "filtered" | "todos";

type TriageTabBarProps = {
  activeTab: TriageTab;
  filteredCount: number;
  importantCount: number;
  onTabChange: (tab: TriageTab) => void;
  todoCount: number;
};

const tabButtonClassName =
  "h-7 rounded-full border border-transparent px-2.5 text-2xs font-medium text-muted-foreground data-[active=true]:border-border/70 data-[active=true]:bg-background/80 data-[active=true]:text-foreground data-[active=true]:shadow-xs";

function TabCount({ value }: { value: number }) {
  if (value === 0) return null;
  return (
    <span className="ml-1 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-muted px-1 text-2xs font-medium text-muted-foreground">
      {value}
    </span>
  );
}

export function TriageTabBar({
  activeTab,
  filteredCount,
  importantCount,
  onTabChange,
  todoCount,
}: TriageTabBarProps) {
  const tabs: ReadonlyArray<{ id: TriageTab; count: number; label: string }> = [
    { id: "important", count: importantCount, label: "Important" },
    { id: "filtered", count: filteredCount, label: "Filtered" },
    { id: "todos", count: todoCount, label: "Todos" },
  ];

  return (
    <div
      aria-label="Triage sections"
      className="flex items-center gap-1"
      role="tablist"
    >
      {tabs.map((tab) => (
        <Button
          aria-selected={activeTab === tab.id}
          className={tabButtonClassName}
          data-active={activeTab === tab.id}
          data-testid={`triage-tab-${tab.id}`}
          key={tab.id}
          onClick={() => onTabChange(tab.id)}
          role="tab"
          size="sm"
          type="button"
          variant="ghost"
        >
          {tab.label}
          <TabCount value={tab.count} />
        </Button>
      ))}
    </div>
  );
}
