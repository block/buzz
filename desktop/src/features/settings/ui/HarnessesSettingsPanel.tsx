import * as React from "react";
import { ExternalLink, Plus, RefreshCw } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  useAcpRuntimesQuery,
  useGitBashPrerequisiteQuery,
} from "@/features/agents/hooks";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { SectionHeader } from "@/shared/ui/PageHeader";

import { HarnessCatalogDialog } from "./HarnessCatalogDialog";
import { HarnessRow } from "./HarnessRow";
import { stableRowOrder, yourHarnessEntries } from "./harnessCatalogLogic";

function GitBashCard({
  prerequisite,
}: {
  prerequisite: NonNullable<
    ReturnType<typeof useGitBashPrerequisiteQuery>["data"]
  >;
}) {
  return (
    <div
      className={cn(
        "min-h-16 rounded-2xl border px-4 py-4 text-sm",
        prerequisite.available
          ? "border-border/60 bg-muted/20"
          : "border-amber-500/20 bg-amber-500/5",
      )}
      data-testid="doctor-git-bash"
    >
      <div className="min-w-0">
        <div className="flex flex-wrap items-center justify-between gap-4">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <p className="text-sm font-medium">Git Bash</p>
            <span aria-hidden="true" className="text-muted-foreground/50">
              ·
            </span>
            <span
              className={cn(
                "inline-flex shrink-0 items-center rounded-md px-2 py-0.5 text-xs font-medium",
                prerequisite.available
                  ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400"
                  : "bg-amber-500/15 text-amber-600 dark:text-amber-400",
              )}
            >
              {prerequisite.available ? "Available" : "Action needed"}
            </span>
          </div>
          {!prerequisite.available ? (
            <button
              className="inline-flex shrink-0 items-center gap-1 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
              onClick={() => void openUrl(prerequisite.installInstructionsUrl)}
              type="button"
            >
              <ExternalLink className="h-4 w-4" /> Install Git for Windows
            </button>
          ) : null}
        </div>
        {!prerequisite.available ? (
          <div className="mt-3 space-y-1 text-sm text-muted-foreground">
            <p>Required for buzz-agent shell tools on Windows.</p>
            <p>{prerequisite.installHint}</p>
          </div>
        ) : null}
      </div>
    </div>
  );
}

/**
 * Consolidated "Agent runtimes" surface for Settings → Agents.
 *
 * Replaces the old "Agent runtimes" (DoctorSettingsPanel) + "Bring your own
 * harness" (HarnessManagementCard) pair with one operational area:
 *
 * - **Your runtimes** — stable rows for ready (or one-click-ready) runtimes
 *   and everything the user authored. Row order never changes when a runtime
 *   installs (stableRowOrder), so the page doesn't jump under the pointer.
 * - **Add runtimes** — a master-detail catalog dialog for everything that
 *   needs multi-step setup, plus the custom-harness form.
 */
export function HarnessesSettingsPanel() {
  const runtimesQuery = useAcpRuntimesQuery();
  const gitBashQuery = useGitBashPrerequisiteQuery();
  const [catalogOpen, setCatalogOpen] = React.useState(false);
  // Incremented each time the user clicks "Check again" so HarnessRow
  // useEffect clears stale install results from before the refresh.
  const [resetEpoch, setResetEpoch] = React.useState(0);

  const entries = React.useMemo(
    () => yourHarnessEntries(runtimesQuery.data ?? []),
    [runtimesQuery.data],
  );

  // Sticky row order: initial sort once, then preserve relative order across
  // refetches/toggles so enabling a harness never reorders the list.
  const orderRef = React.useRef<string[]>([]);
  const rows = React.useMemo(() => {
    orderRef.current = stableRowOrder(orderRef.current, entries);
    const byId = new Map(entries.map((e) => [e.id, e]));
    return orderRef.current
      .map((id) => byId.get(id))
      .filter((e): e is AcpRuntimeCatalogEntry => e !== undefined);
  }, [entries]);

  const isRefreshing = runtimesQuery.isFetching;

  return (
    <section className="min-w-0 space-y-4" data-testid="settings-harnesses">
      <SectionHeader
        className="items-center"
        title="Agent runtimes"
        description="Choose which agent tools Buzz can use on this device."
        action={
          <Button
            disabled={isRefreshing}
            onClick={() => {
              setResetEpoch((e) => e + 1);
              void runtimesQuery.refetch();
              void gitBashQuery.refetch();
            }}
            size="sm"
            type="button"
            variant="outline"
          >
            <RefreshCw
              className={cn("h-4 w-4", isRefreshing && "animate-spin")}
            />
            Check again
          </Button>
        }
      />

      <div className="space-y-8">
        {gitBashQuery.data ? (
          <section>
            <div className="mb-3 text-sm">
              <h2 className="text-lg font-semibold tracking-tight">
                System prerequisites
              </h2>
              <p className="mt-1 text-sm font-normal text-muted-foreground">
                Windows tools required by supported agents.
              </p>
            </div>
            <GitBashCard prerequisite={gitBashQuery.data} />
          </section>
        ) : null}

        <section aria-label="Your runtimes">
          {/* The sub-header only earns its keep when another section (System
              prerequisites, Windows-only) shares the page; otherwise it just
              restates the page header. */}
          {gitBashQuery.data ? (
            <div className="mb-3 text-sm">
              <h2 className="text-lg font-semibold tracking-tight">
                Your runtimes
              </h2>
              <p className="mt-1 text-sm font-normal text-muted-foreground">
                Ready to use, or one click from installed.
              </p>
            </div>
          ) : null}

          {runtimesQuery.isLoading ? (
            <div className="rounded-2xl bg-muted/20 px-4 py-4 text-sm font-normal text-muted-foreground">
              Checking agent runtimes...
            </div>
          ) : rows.length > 0 ? (
            <div className="space-y-3" data-testid="doctor-runtime-list">
              {rows.map((runtime) => (
                <HarnessRow
                  key={runtime.id}
                  resetEpoch={resetEpoch}
                  runtime={runtime}
                />
              ))}
            </div>
          ) : (
            <div className="rounded-2xl bg-amber-500/10 px-4 py-4 text-sm text-warning">
              No agent runtimes ready yet — add one below.
            </div>
          )}

          {runtimesQuery.error instanceof Error ? (
            <p className="mt-3 rounded-2xl bg-destructive/10 px-4 py-4 text-sm text-destructive">
              {runtimesQuery.error.message}
            </p>
          ) : null}

          <Button
            className="mt-3 gap-2"
            data-testid="harness-add-button"
            onClick={() => setCatalogOpen(true)}
            size="sm"
            type="button"
            variant="outline"
          >
            <Plus className="h-4 w-4" />
            Add runtimes
          </Button>
        </section>
      </div>

      <HarnessCatalogDialog onOpenChange={setCatalogOpen} open={catalogOpen} />
    </section>
  );
}
