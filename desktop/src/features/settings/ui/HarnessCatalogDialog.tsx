import * as React from "react";
import { ChevronRight, ExternalLink, Plus, Search } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  useAcpRuntimesQuery,
  useInstallAcpRuntimeMutation,
} from "@/features/agents/hooks";
import { useInstallOutputLine } from "@/features/agents/lib/useInstallOutputLine";
import {
  getRuntimeDisplayLabel,
  RuntimeIcon,
} from "@/features/onboarding/ui/RuntimeIcon";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { getInstallErrorMessage } from "@/shared/lib/installError";
import { cn } from "@/shared/lib/cn";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";
import { Spinner } from "@/shared/ui/spinner";

import { CustomHarnessForm } from "./CustomHarnessForm";
import { harnessDescription } from "./harnessCatalogCopy";
import {
  adapterUpdateWarning,
  catalogDialogEntries,
  catalogPrimaryAction,
  entryStatusLabel,
  filterCatalogEntries,
  groupCatalogEntries,
  installLinkLabel,
} from "./harnessCatalogLogic";

/** Sentinel list selection for the "+ Custom harness" entry. */
const CUSTOM_ENTRY_ID = "\u0000custom";

/**
 * "Add runtimes" — master-detail catalog dialog, modeled on the Agent
 * Catalog (PersonaCatalogDialog): searchable left chooser, right detail pane
 * with one neutral vendor-sourced sentence, operational setup state, and
 * technical details, plus a primary Install / setup-guide CTA pinned in a
 * bottom action bar (same position as the custom-harness Save button).
 * "+ Custom harness" opens the progressive-disclosure form in the same pane.
 */
export function HarnessCatalogDialog({
  onOpenChange,
  open,
}: {
  onOpenChange: (open: boolean) => void;
  open: boolean;
}) {
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const runtimesQuery = useAcpRuntimesQuery();
  const isLoading = runtimesQuery.isLoading;
  const entries = React.useMemo(
    () => catalogDialogEntries(runtimesQuery.data ?? []),
    [runtimesQuery.data],
  );
  const [query, setQuery] = React.useState("");
  const filtered = React.useMemo(
    () => filterCatalogEntries(entries, query),
    [entries, query],
  );
  const [selectedId, setSelectedId] = React.useState<string | null>(null);
  const groups = React.useMemo(() => groupCatalogEntries(filtered), [filtered]);
  // "Installed" is collapsed by default — the dialog exists to set up new
  // runtimes. Searching force-expands it (VS Code-style) so matches are never
  // hidden; likewise when there is nothing left to set up.
  const [setupOpen, setSetupOpen] = React.useState(true);
  const [installedOpen, setInstalledOpen] = React.useState(false);
  const isSearching = query.trim().length > 0;
  const setupExpanded = setupOpen || isSearching;
  const installedExpanded =
    installedOpen || isSearching || groups.setup.length === 0;

  // Keep a valid selection: default to the first visible entry; hold on to
  // the custom-form selection regardless of the filter.
  React.useEffect(() => {
    if (!open) return;
    setSelectedId((current) => {
      if (current === CUSTOM_ENTRY_ID) return current;
      if (current && filtered.some((e) => e.id === current)) return current;
      return filtered[0]?.id ?? null;
    });
  }, [open, filtered]);

  // Reset transient state when the dialog closes.
  React.useEffect(() => {
    if (!open) {
      setQuery("");
      setSelectedId(null);
      setSetupOpen(true);
      setInstalledOpen(false);
    }
  }, [open]);

  const selectedEntry =
    selectedId === CUSTOM_ENTRY_ID
      ? null
      : (filtered.find((e) => e.id === selectedId) ?? null);

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <ChooserDialogContent
        className="h-[42rem] max-w-4xl"
        contentClassName="flex min-h-0 flex-1 p-0"
        data-testid="harness-catalog-dialog"
        headerClassName="bg-sidebar pb-3 text-sidebar-foreground"
        headerTestId="harness-catalog-dialog-header"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          contentRef.current?.focus();
        }}
        ref={contentRef}
        scrollAreaClassName="flex min-h-0 overflow-hidden px-0"
        scrollAreaTestId="harness-catalog-dialog-body"
        tabIndex={-1}
        title="Add runtimes"
      >
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-sidebar sm:flex-row">
          {/* Left: search + chooser list */}
          <div className="flex max-h-56 min-h-0 flex-col sm:max-h-none sm:w-56">
            <div className="px-3 pt-3">
              <div className="relative">
                <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-sidebar-foreground/50" />
                <Input
                  aria-label="Search runtimes"
                  className="h-8 border-sidebar-border bg-sidebar-accent/40 pl-8 text-sm"
                  data-testid="harness-catalog-search"
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Search runtimes…"
                  value={query}
                />
              </div>
            </div>
            <div
              className="min-h-0 flex-1 overflow-y-auto px-2 py-3"
              data-testid="harness-catalog-list"
            >
              <div className="space-y-1">
                {isLoading ? (
                  <CatalogListSkeleton />
                ) : filtered.length === 0 ? (
                  <p className="px-4 py-2 text-sm text-sidebar-foreground/60">
                    No runtimes match.
                  </p>
                ) : (
                  <>
                    {groups.setup.length > 0 ? (
                      <CatalogSection
                        count={groups.setup.length}
                        label="Setup"
                        onToggle={() => setSetupOpen((v) => !v)}
                        open={setupExpanded}
                        testId="harness-catalog-section-setup"
                      >
                        {groups.setup.map((entry) => (
                          <CatalogListItem
                            entry={entry}
                            isCurrent={entry.id === selectedId}
                            key={entry.id}
                            onSelect={() => setSelectedId(entry.id)}
                          />
                        ))}
                      </CatalogSection>
                    ) : null}
                    {groups.installed.length > 0 ? (
                      <CatalogSection
                        count={groups.installed.length}
                        label="Installed"
                        onToggle={() => setInstalledOpen((v) => !v)}
                        open={installedExpanded}
                        testId="harness-catalog-section-installed"
                      >
                        {groups.installed.map((entry) => (
                          <CatalogListItem
                            entry={entry}
                            isCurrent={entry.id === selectedId}
                            key={entry.id}
                            onSelect={() => setSelectedId(entry.id)}
                          />
                        ))}
                      </CatalogSection>
                    ) : null}
                  </>
                )}
              </div>
              <div className="my-2 border-t border-sidebar-border/60" />
              <button
                aria-current={
                  selectedId === CUSTOM_ENTRY_ID ? "true" : undefined
                }
                className={cn(
                  "flex w-full items-center gap-2 rounded-lg px-4 py-1.5 text-left transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
                  selectedId === CUSTOM_ENTRY_ID
                    ? "bg-sidebar-active text-sidebar-active-foreground"
                    : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                )}
                data-testid="harness-catalog-list-item-custom"
                onClick={() => setSelectedId(CUSTOM_ENTRY_ID)}
                type="button"
              >
                <Plus className="h-4 w-4 shrink-0" />
                <span className="min-w-0 flex-1 truncate text-sm font-medium">
                  Custom harness
                </span>
              </button>
            </div>
          </div>

          {/* Right: detail pane */}
          <div className="relative z-10 ml-px flex min-h-0 flex-1 flex-col overflow-hidden rounded-tl-xl bg-background shadow-[-1px_0_0_0_hsl(var(--sidebar-border)/0.45)]">
            <div
              className="flex min-h-0 flex-1 flex-col overflow-hidden"
              data-testid="harness-catalog-detail-pane"
            >
              {selectedId === CUSTOM_ENTRY_ID ? (
                <CustomHarnessDetail onDone={() => onOpenChange(false)} />
              ) : selectedEntry ? (
                <CatalogDetail entry={selectedEntry} key={selectedEntry.id} />
              ) : isLoading ? (
                <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
                  <CatalogDetailSkeleton />
                </div>
              ) : (
                <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
                  <div className="flex min-h-80 items-center justify-center rounded-lg border border-dashed border-border/70 px-6 text-center">
                    <p className="max-w-sm text-sm text-muted-foreground">
                      Select a runtime on the left, or add a custom one.
                    </p>
                  </div>
                </div>
              )}
            </div>
          </div>
        </div>
      </ChooserDialogContent>
    </Dialog>
  );
}

/**
 * VS Code-style collapsible list section: chevron + uppercase label +
 * trailing count badge. The header is a plain toggle — selection lives on
 * the rows inside.
 */
function CatalogSection({
  children,
  count,
  label,
  onToggle,
  open,
  testId,
}: {
  children: React.ReactNode;
  count: number;
  label: string;
  onToggle: () => void;
  open: boolean;
  testId: string;
}) {
  return (
    <div>
      <button
        aria-expanded={open}
        className="flex w-full items-center gap-1 rounded-md px-2 py-1 text-left text-2xs font-semibold uppercase tracking-wider text-sidebar-foreground/60 transition-colors hover:text-sidebar-foreground focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50"
        data-testid={testId}
        onClick={onToggle}
        type="button"
      >
        <ChevronRight
          aria-hidden
          className={cn(
            "h-3 w-3 shrink-0 transition-transform",
            open && "rotate-90",
          )}
        />
        <span className="min-w-0 flex-1 truncate">{label}</span>
        <span
          className="ml-auto shrink-0 rounded-full bg-sidebar-accent px-1.5 py-px font-medium normal-case tracking-normal text-sidebar-foreground/70"
          data-testid={`${testId}-count`}
        >
          {count}
        </span>
      </button>
      {open ? <div className="space-y-1 pt-0.5">{children}</div> : null}
    </div>
  );
}

/** Pulsing placeholder rows shown while harness discovery is running. */
function CatalogListSkeleton() {
  const widths = ["w-24", "w-32", "w-20", "w-28", "w-24", "w-16"];
  return (
    <div
      aria-label="Loading runtimes"
      className="space-y-1"
      data-testid="harness-catalog-list-skeleton"
      role="status"
    >
      {widths.map((width, index) => (
        <div
          className="flex w-full items-center gap-2 rounded-lg px-4 py-1.5"
          // Static placeholder list — order never changes.
          // biome-ignore lint/suspicious/noArrayIndexKey: static skeleton rows
          key={index}
        >
          <Skeleton className="h-6 w-6 shrink-0 rounded-md" />
          <Skeleton className={cn("h-3.5 rounded", width)} />
        </div>
      ))}
    </div>
  );
}

/** Pulsing placeholder mirroring the detail-pane layout while loading. */
function CatalogDetailSkeleton() {
  return (
    <div
      aria-label="Loading runtime details"
      className="flex min-h-full flex-col gap-6"
      data-testid="harness-catalog-detail-skeleton"
      role="status"
    >
      <div className="flex items-center gap-4">
        <Skeleton className="h-16 w-16 rounded-xl" />
        <div className="min-w-0 space-y-2">
          <Skeleton className="h-5 w-40 rounded" />
          <Skeleton className="h-4 w-16 rounded" />
          <Skeleton className="h-7 w-24 rounded-md" />
        </div>
      </div>
      <div className="space-y-2">
        <Skeleton className="h-3.5 w-full rounded" />
        <Skeleton className="h-3.5 w-11/12 rounded" />
        <Skeleton className="h-3.5 w-2/3 rounded" />
      </div>
      <Skeleton className="h-28 w-full rounded-lg" />
    </div>
  );
}

function CatalogListItem({
  entry,
  isCurrent,
  onSelect,
}: {
  entry: AcpRuntimeCatalogEntry;
  isCurrent: boolean;
  onSelect: () => void;
}) {
  const isReady = entry.availability === "available";

  return (
    <button
      aria-current={isCurrent ? "true" : undefined}
      className={cn(
        "flex w-full items-center gap-2 rounded-lg px-4 py-1.5 text-left transition-[background-color,color,box-shadow] focus:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring/50 focus-visible:ring-offset-2 focus-visible:ring-offset-sidebar",
        isCurrent
          ? "bg-sidebar-active text-sidebar-active-foreground"
          : "text-sidebar-foreground/70 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
      )}
      data-testid={`harness-catalog-list-item-${entry.id}`}
      onClick={onSelect}
      type="button"
    >
      <RuntimeIcon className="h-6 w-6" runtime={entry} />
      <span className="min-w-0 flex-1 truncate text-sm font-medium">
        {getRuntimeDisplayLabel(entry)}
      </span>
      {isReady ? (
        <span
          aria-label={`${entry.label} is ready`}
          className="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500"
          role="img"
        />
      ) : null}
    </button>
  );
}

function CatalogDetail({ entry }: { entry: AcpRuntimeCatalogEntry }) {
  const install = useInstallAcpRuntimeMutation();
  const [installError, setInstallError] = React.useState<string | null>(null);
  const [isUpdateWarningOpen, setIsUpdateWarningOpen] = React.useState(false);
  const action = catalogPrimaryAction(entry);
  const statusLabel = entryStatusLabel(entry);
  const description = harnessDescription(entry.id);
  const isReady = entry.availability === "available";
  const docsUrl = entry.installInstructionsUrl.trim();
  const installOutputLine = useInstallOutputLine(entry.id, install.isPending);

  function handleInstall() {
    setInstallError(null);
    install.mutate(entry.id, {
      onSuccess: (result) => {
        if (!result.success) {
          setInstallError(getInstallErrorMessage(result));
        }
      },
      onError: (error) => {
        setInstallError(
          error instanceof Error ? error.message : "Install failed.",
        );
      },
    });
  }

  // Replacing an outdated adapter is a machine-wide mutation — gate it behind
  // the same confirmation the runtime row shows (adapterUpdateWarning is the
  // shared copy source). Fresh installs stay one-click.
  function handlePrimaryClick() {
    if (entry.availability === "adapter_outdated") {
      setIsUpdateWarningOpen(true);
      return;
    }
    handleInstall();
  }

  // The outline docs button only shows when the entry needs no setup action
  // (already ready) — pair it with a hint so the muted styling reads as
  // "nothing to do here" rather than a broken primary button.
  const isSecondaryCta = action.kind !== "install" && action.kind !== "docs";

  const primaryCta =
    action.kind === "install" ? (
      <Button
        data-testid={`harness-catalog-install-${entry.id}`}
        disabled={install.isPending}
        onClick={handlePrimaryClick}
        type="button"
      >
        {install.isPending ? <Spinner className="mr-2 h-3.5 w-3.5" /> : null}
        {action.label}
      </Button>
    ) : docsUrl ? (
      <Button
        data-testid={
          action.kind === "docs"
            ? `harness-catalog-setup-${entry.id}`
            : `harness-catalog-docs-${entry.id}`
        }
        onClick={() => void openUrl(docsUrl)}
        type="button"
        variant={action.kind === "docs" ? "default" : "outline"}
      >
        <ExternalLink className="mr-1 h-3.5 w-3.5" />
        {action.kind === "docs" ? action.label : installLinkLabel(entry)}
      </Button>
    ) : null;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-5 py-5">
        <div className="flex items-center gap-4">
          <RuntimeIcon className="h-16 w-16" runtime={entry} />
          <div className="min-w-0">
            <h3 className="truncate text-xl font-semibold leading-snug">
              {getRuntimeDisplayLabel(entry)}
            </h3>
            {statusLabel ? (
              // entryStatusLabel is the single availability→label source
              // shared with the row chip — when it has something to say
              // (setup needed, sign-in needed, config error) it outranks the
              // green Ready chip even for available entries.
              <span
                className="mt-1 inline-flex items-center rounded-md bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground"
                data-testid={`harness-catalog-status-${entry.id}`}
              >
                {statusLabel}
              </span>
            ) : isReady ? (
              <span className="mt-1 inline-flex items-center rounded-md bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
                Ready
              </span>
            ) : null}
          </div>
        </div>

        {description ? (
          <p className="text-sm leading-6 text-muted-foreground">
            {description}
          </p>
        ) : null}

        {entry.installHint ? (
          <div className="space-y-1">
            <p className="text-xs font-semibold text-muted-foreground">Setup</p>
            <p className="whitespace-pre-line text-sm leading-6 text-foreground">
              {entry.installHint}
            </p>
          </div>
        ) : null}

        {install.isPending && installOutputLine ? (
          <p
            aria-live="polite"
            className="truncate rounded-lg border border-border/60 bg-muted/40 px-3 py-1.5 font-mono text-xs text-muted-foreground"
            data-testid={`harness-catalog-install-output-${entry.id}`}
          >
            {installOutputLine}
          </p>
        ) : null}

        {installError ? (
          <p className="whitespace-pre-line rounded-lg border border-destructive/30 bg-destructive/10 px-3 py-1.5 text-sm text-destructive">
            {installError}
          </p>
        ) : null}

        <TechnicalDetails entry={entry} />
      </div>

      {primaryCta ? (
        <div className="flex shrink-0 items-center justify-end gap-3 border-t border-border/60 bg-background px-5 py-3">
          {isSecondaryCta ? (
            <p
              className="text-xs text-muted-foreground/80"
              data-testid={`harness-catalog-ready-hint-${entry.id}`}
            >
              Already set up
            </p>
          ) : null}
          {primaryCta}
        </div>
      ) : null}
      <AlertDialog
        onOpenChange={setIsUpdateWarningOpen}
        open={isUpdateWarningOpen}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Update {entry.label} adapter?</AlertDialogTitle>
            <AlertDialogDescription>
              {adapterUpdateWarning(entry)}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              data-testid={`harness-catalog-confirm-update-${entry.id}`}
              onClick={handleInstall}
            >
              Update
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function TechnicalDetails({ entry }: { entry: AcpRuntimeCatalogEntry }) {
  const rows: Array<{ label: string; value: string }> = [
    { label: "ID", value: entry.id },
    ...(entry.command ? [{ label: "Command", value: entry.command }] : []),
    ...(entry.defaultArgs.length > 0
      ? [{ label: "Arguments", value: entry.defaultArgs.join(" ") }]
      : []),
    ...(entry.underlyingCliPath
      ? [{ label: "Underlying CLI", value: entry.underlyingCliPath }]
      : []),
    ...(entry.binaryPath ? [{ label: "Path", value: entry.binaryPath }] : []),
    {
      label: "Source",
      value: entry.source === "builtin" ? "Built-in" : "Bundled preset",
    },
  ];

  return (
    <dl
      className="space-y-1.5 rounded-lg border border-border/70 bg-card/70 px-4 py-3"
      data-testid={`harness-catalog-technical-${entry.id}`}
    >
      {rows.map((row) => (
        <div className="flex gap-3 text-sm" key={row.label}>
          <dt className="w-28 shrink-0 text-xs font-semibold leading-5 text-muted-foreground">
            {row.label}
          </dt>
          <dd className="min-w-0 break-all font-mono text-xs leading-5">
            {row.value}
          </dd>
        </div>
      ))}
    </dl>
  );
}

function CustomHarnessDetail({ onDone }: { onDone: () => void }) {
  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      data-testid="harness-catalog-custom-detail"
    >
      <CustomHarnessForm
        chromeless
        header={
          <div>
            <h3 className="text-xl font-semibold leading-snug">
              Custom harness
            </h3>
            <p className="mt-1 text-sm text-muted-foreground">
              Register any ACP-speaking agent tool as a selectable runtime.
            </p>
          </div>
        }
        onCancel={onDone}
        onSaved={onDone}
      />
    </div>
  );
}
