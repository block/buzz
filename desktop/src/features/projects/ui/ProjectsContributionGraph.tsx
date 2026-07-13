import { cn } from "@/shared/lib/cn";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

const WEEK_COUNT = 26;
const DAYS_PER_WEEK = 7;

// Intensity ramp shared by the cells and the "Less … More" legend.
// Grayscale on the theme's foreground token: the most active cells are
// near-black in light mode and near-white in dark mode.
const LEVEL_CLASSES = [
  "bg-muted/60 dark:bg-muted/40",
  "bg-foreground/25",
  "bg-foreground/50",
  "bg-foreground/75",
  "bg-foreground",
];

function dayKeyOf(date: Date) {
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

function levelFor(count: number) {
  if (count <= 0) return 0;
  if (count <= 2) return 1;
  if (count <= 5) return 2;
  if (count <= 9) return 3;
  return 4;
}

/** Week columns (each 7 days, Sunday first) ending with the current week. */
function buildWeeks(today: Date) {
  const start = new Date(
    today.getFullYear(),
    today.getMonth(),
    today.getDate() - today.getDay() - (WEEK_COUNT - 1) * DAYS_PER_WEEK,
  );

  return Array.from({ length: WEEK_COUNT }, (_, weekIndex) =>
    Array.from({ length: DAYS_PER_WEEK }, (_, dayIndex) => {
      const date = new Date(start);
      date.setDate(start.getDate() + weekIndex * DAYS_PER_WEEK + dayIndex);
      return date;
    }),
  );
}

function monthLabels(weeks: Date[][]) {
  return weeks.map((week, index) => {
    if (index === 0) return "";
    const month = week[0].getMonth();
    return month !== weeks[index - 1][0].getMonth()
      ? week[0].toLocaleDateString(undefined, { month: "short" })
      : "";
  });
}

/**
 * GitHub-style activity heatmap for the last 26 weeks, fed by per-day
 * activity counts (see `ProjectActivitySummary.activityByDay`).
 */
export function ProjectsContributionGraph({
  activityByDay,
  className,
}: {
  activityByDay: Record<string, number>;
  className?: string;
}) {
  const today = new Date();
  const weeks = buildWeeks(today);
  const labels = monthLabels(weeks);
  const gridTemplateColumns = `repeat(${weeks.length}, minmax(0, 1fr))`;
  const todayKey = dayKeyOf(today);

  let totalContributions = 0;
  for (const week of weeks) {
    for (const day of week) {
      const key = dayKeyOf(day);
      if (key > todayKey) continue;
      totalContributions += activityByDay[key] ?? 0;
    }
  }

  return (
    <div className={cn("space-y-2", className)}>
      <div className="grid gap-2" style={{ gridTemplateColumns }}>
        {labels.map((label, index) => (
          <span
            className="overflow-visible whitespace-nowrap text-2xs font-medium text-muted-foreground"
            // Columns are positional; labels have no stable content key.
            // biome-ignore lint/suspicious/noArrayIndexKey: fixed-size grid
            key={index}
          >
            {label}
          </span>
        ))}
      </div>
      <div
        className="grid grid-flow-col grid-rows-7 gap-2"
        style={{ gridTemplateColumns }}
      >
        {weeks.map((week) =>
          week.map((day) => {
            const key = dayKeyOf(day);
            if (key > todayKey) {
              return <span aria-hidden className="aspect-square" key={key} />;
            }
            const count = activityByDay[key] ?? 0;
            const dateLabel = day.toLocaleDateString(undefined, {
              month: "short",
              day: "numeric",
            });
            return (
              <Tooltip key={key}>
                <TooltipTrigger asChild>
                  <span
                    className={cn(
                      "aspect-square w-full rounded-[3px]",
                      LEVEL_CLASSES[levelFor(count)],
                    )}
                  />
                </TooltipTrigger>
                <TooltipContent>
                  {count > 0
                    ? `${count} ${count === 1 ? "event" : "events"} · ${dateLabel}`
                    : `No activity · ${dateLabel}`}
                </TooltipContent>
              </Tooltip>
            );
          }),
        )}
      </div>
      <div className="flex items-center justify-between gap-2 pt-1">
        <p className="text-xs text-muted-foreground">
          {totalContributions} {totalContributions === 1 ? "event" : "events"}{" "}
          in the last 6 months
        </p>
        <div className="flex items-center gap-1.5 text-2xs text-muted-foreground">
          Less
          {LEVEL_CLASSES.map((levelClass) => (
            <span
              className={cn("h-2.5 w-2.5 rounded-[3px]", levelClass)}
              key={levelClass}
            />
          ))}
          More
        </div>
      </div>
    </div>
  );
}
