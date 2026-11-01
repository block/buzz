import * as React from "react";

import {
  indexCatalog,
  type UsageWindowDays,
} from "@/features/agents/lib/modelDashboard";
import {
  useModelLandscape,
  useModelUsageSeries,
} from "@/features/agents/modelDashboardHooks";
import { SectionHeader } from "@/shared/ui/PageHeader";
import { SegmentedControl } from "@/shared/ui/segmented-control";
import { ModelLandscapePanel } from "./ModelLandscapePanel";
import { ModelUsagePanel } from "./ModelUsagePanel";

type ModelsTab = "usage" | "landscape";

const TAB_OPTIONS = [
  { value: "usage", label: "Your usage" },
  { value: "landscape", label: "Landscape" },
] as const;

const WINDOW_OPTIONS = [
  { value: "7", label: "7 days" },
  { value: "30", label: "30 days" },
] as const;

/**
 * Models dashboard on the Agents page: what your agents spent on frontier vs
 * open-weight/local models, next to the public model landscape.
 */
export function ModelsSection() {
  const [tab, setTab] = React.useState<ModelsTab>("usage");
  const [windowDays, setWindowDays] = React.useState<UsageWindowDays>(7);
  // Load the catalog for both tabs once either is viewed: usage uses it to
  // classify ids the family prefixes miss.
  const [catalogRequested, setCatalogRequested] = React.useState(false);

  const usageQuery = useModelUsageSeries(windowDays);
  const landscapeQuery = useModelLandscape(
    catalogRequested || tab === "landscape",
  );
  const catalog = React.useMemo(
    () => (landscapeQuery.data ? indexCatalog(landscapeQuery.data) : undefined),
    [landscapeQuery.data],
  );

  React.useEffect(() => {
    if (usageQuery.data && usageQuery.data.coverage.reportCount > 0) {
      setCatalogRequested(true);
    }
  }, [usageQuery.data]);

  return (
    <section className="relative space-y-4" data-testid="agents-models">
      <SectionHeader
        title="Models"
        description="Usage across frontier and local models, and how they compare."
        action={
          <SegmentedControl
            legend="Models view"
            onValueChange={setTab}
            optionTestIdPrefix="agents-models-tab"
            options={TAB_OPTIONS}
            size="compact"
            testId="agents-models-tabs"
            value={tab}
          />
        }
      />

      {tab === "usage" ? (
        <ModelUsagePanel
          catalog={catalog}
          error={usageQuery.error instanceof Error ? usageQuery.error : null}
          isLoading={usageQuery.isLoading}
          series={usageQuery.data}
          windowControl={
            <SegmentedControl
              legend="Usage window"
              onValueChange={(value) =>
                setWindowDays(Number(value) as UsageWindowDays)
              }
              optionTestIdPrefix="agents-models-window"
              options={WINDOW_OPTIONS}
              size="compact"
              testId="agents-models-window"
              value={String(windowDays) as "7" | "30"}
            />
          }
        />
      ) : (
        <ModelLandscapePanel
          error={
            landscapeQuery.error instanceof Error ? landscapeQuery.error : null
          }
          isLoading={landscapeQuery.isLoading}
          models={landscapeQuery.data ?? []}
          series={usageQuery.data}
        />
      )}
    </section>
  );
}
