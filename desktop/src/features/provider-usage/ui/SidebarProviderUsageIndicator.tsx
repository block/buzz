import * as React from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, RefreshCw } from "lucide-react";

import {
  getProviderUsage,
  listProviderUsageCapabilities,
} from "@/shared/api/tauriProviderUsage";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Progress } from "@/shared/ui/progress";
import { Spinner } from "@/shared/ui/spinner";
import { cn } from "@/shared/lib/cn";
import {
  formatTokenCount,
  formatUsageReset,
  providerUsageErrorMessage,
  providerUsageTone,
} from "@/features/provider-usage/providerUsageDisplay.mjs";
import {
  resolveProviderUsagePreference,
  useProviderUsagePreference,
} from "@/features/provider-usage/providerUsagePreference";

const FIVE_MINUTES = 5 * 60 * 1000;

const toneClasses = {
  healthy: {
    progress: "[&>div]:bg-primary",
    sidebarStroke: "stroke-sidebar-primary",
  },
  warning: {
    progress: "[&>div]:bg-warning",
    sidebarStroke: "stroke-warning",
  },
  critical: {
    progress: "[&>div]:bg-destructive",
    sidebarStroke: "stroke-destructive",
  },
} as const;

function UsageRing({
  isLoading,
  remainingPercent,
}: {
  isLoading: boolean;
  remainingPercent?: number;
}) {
  const sizeClass = "h-[18px] w-[18px]";

  if (isLoading) {
    return <Spinner aria-hidden="true" className={cn(sizeClass, "border-2")} />;
  }
  if (remainingPercent === undefined) {
    return (
      <span
        className={cn(
          "flex shrink-0 items-center justify-center rounded-full bg-destructive/10 text-destructive",
          sizeClass,
        )}
      >
        <AlertTriangle aria-hidden="true" className="h-2.5 w-2.5" />
      </span>
    );
  }

  const tone = providerUsageTone(remainingPercent);
  const circumference = 2 * Math.PI * 14;
  const dashOffset = circumference * (1 - remainingPercent / 100);
  return (
    <span className={cn("relative shrink-0", sizeClass)} aria-hidden="true">
      <svg
        aria-hidden="true"
        className={cn("-rotate-90", sizeClass)}
        viewBox="0 0 32 32"
      >
        <circle
          className="stroke-sidebar-border/70"
          cx="16"
          cy="16"
          fill="none"
          r="14"
          strokeWidth="3"
        />
        <circle
          className={cn(
            "motion-safe:transition-[stroke-dashoffset] motion-safe:duration-300",
            toneClasses[tone].sidebarStroke,
          )}
          cx="16"
          cy="16"
          fill="none"
          r="14"
          strokeDasharray={circumference}
          strokeDashoffset={dashOffset}
          strokeLinecap="round"
          strokeWidth="3"
        />
      </svg>
    </span>
  );
}

export function SidebarProviderUsageIndicator({
  placement = "sidebar",
}: {
  placement?: "chrome" | "sidebar";
}) {
  const preference = useProviderUsagePreference();
  const capabilitiesQuery = useQuery({
    queryKey: ["provider-usage-capabilities"],
    queryFn: listProviderUsageCapabilities,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const provider = resolveProviderUsagePreference(
    preference,
    capabilitiesQuery.data,
  );
  const supported = provider === "codex";
  const query = useQuery({
    queryKey: ["provider-usage", provider],
    queryFn: () => getProviderUsage(provider),
    enabled: supported,
    staleTime: FIVE_MINUTES,
    refetchInterval: FIVE_MINUTES,
    refetchIntervalInBackground: false,
    refetchOnWindowFocus: true,
    retry: 1,
  });
  const compact = placement === "chrome";

  const usage = query.data;
  const constrainingWindow = usage?.windows.reduce((lowest, window) =>
    window.remainingPercent < lowest.remainingPercent ? window : lowest,
  );
  const remainingPercent = constrainingWindow?.remainingPercent;
  const tone =
    remainingPercent === undefined
      ? undefined
      : providerUsageTone(remainingPercent);
  const productLabel =
    provider === "codex" ? "Codex" : provider === "claude" ? "Claude" : "Grok";
  const planLabel =
    usage?.planType === undefined || usage.planType === null
      ? productLabel
      : `${productLabel} ${usage.planType.charAt(0).toUpperCase()}${usage.planType.slice(1)}`;
  const unsupportedMessage = supported
    ? null
    : `${productLabel} personal allowance is not supported yet`;
  const errorMessage = unsupportedMessage
    ? unsupportedMessage
    : query.isError
      ? providerUsageErrorMessage(query.error)
      : null;
  const popoverTitleId = React.useId();
  const popoverDescriptionId = React.useId();
  const allowanceHeadingId = React.useId();
  const [refreshNotice, setRefreshNotice] = React.useState("");
  const additionalWindows =
    usage?.windows.filter((window) => window.id !== constrainingWindow?.id) ??
    [];
  const triggerLabel =
    remainingPercent !== undefined
      ? `${planLabel}: ${remainingPercent}% left`
      : (errorMessage ?? `Loading ${productLabel} allowance`);

  async function refreshAllowance() {
    setRefreshNotice("");
    const result = await query.refetch();
    setRefreshNotice(
      result.isSuccess
        ? `${productLabel} allowance updated.`
        : `${productLabel} allowance could not be updated.`,
    );
  }

  return (
    <Popover>
      <PopoverTrigger asChild>
        {compact ? (
          <ChromeUsageTrigger
            errorMessage={errorMessage}
            isLoading={query.isPending && supported}
            productLabel={productLabel}
            remainingPercent={remainingPercent}
            triggerLabel={triggerLabel}
          />
        ) : (
          <SettingsUsageTrigger
            errorMessage={errorMessage}
            isLoading={query.isPending && supported}
            productLabel={productLabel}
            remainingPercent={remainingPercent}
            triggerLabel={triggerLabel}
          />
        )}
      </PopoverTrigger>

      <PopoverContent
        aria-describedby={popoverDescriptionId}
        aria-labelledby={popoverTitleId}
        align="end"
        className="max-h-[min(38rem,var(--radix-popover-content-available-height))] w-[min(25rem,var(--radix-popover-content-available-width),calc(100vw-1.5rem))] overflow-y-auto overscroll-contain p-3"
        collisionPadding={12}
        role="dialog"
        side={compact ? "bottom" : "right"}
        sideOffset={compact ? 10 : 14}
      >
        <div className="px-0.5">
          <h2 className="text-sm font-semibold" id={popoverTitleId}>
            AI usage
          </h2>
          <p
            className="mt-0.5 text-2xs text-muted-foreground"
            id={popoverDescriptionId}
          >
            Personal provider allowance
          </p>
        </div>

        <section
          aria-labelledby={allowanceHeadingId}
          className="mt-3 rounded-lg border border-border/70 p-2.5"
        >
          <div className="flex min-w-0 items-start justify-between gap-2">
            <div className="min-w-0">
              <h3
                className="truncate text-sm font-semibold"
                id={allowanceHeadingId}
              >
                {planLabel}
              </h3>
              <p className="mt-0.5 text-2xs text-muted-foreground">
                Personal subscription allowance
              </p>
            </div>
            {supported ? (
              <Button
                aria-label={`Refresh ${productLabel} allowance`}
                disabled={query.isFetching}
                onClick={() => void refreshAllowance()}
                size="icon"
                type="button"
                variant="ghost"
              >
                <RefreshCw
                  aria-hidden="true"
                  className={cn(query.isFetching && "motion-safe:animate-spin")}
                />
              </Button>
            ) : null}
          </div>
          <span aria-live="polite" className="sr-only" role="status">
            {refreshNotice}
          </span>

          {usage && constrainingWindow ? (
            <div className="mt-2.5 space-y-2.5">
              {query.isError ? (
                <div className="rounded-md bg-warning-bg px-2 py-1.5 text-2xs text-warning">
                  Last successful data shown; the latest refresh failed.
                </div>
              ) : null}
              <div>
                <div className="mb-1.5 flex items-baseline justify-between gap-3">
                  <span className="text-lg font-semibold tabular-nums">
                    {constrainingWindow.remainingPercent}% remaining
                  </span>
                  <span className="text-xs text-muted-foreground">
                    {constrainingWindow.usedPercent}% used
                  </span>
                </div>
                <Progress
                  aria-label={`${productLabel}: ${constrainingWindow.remainingPercent}% remaining`}
                  aria-valuetext={`${constrainingWindow.remainingPercent}% remaining for ${constrainingWindow.label}; resets ${formatUsageReset(constrainingWindow.resetsAt)}`}
                  className={cn(
                    "h-1.5 bg-muted [&>div]:motion-reduce:transition-none",
                    tone && toneClasses[tone].progress,
                  )}
                  value={constrainingWindow.remainingPercent}
                />
                <p className="mt-1.5 text-2xs text-muted-foreground">
                  {constrainingWindow.label} · Resets{" "}
                  {formatUsageReset(constrainingWindow.resetsAt)}
                </p>
              </div>
              {additionalWindows.length > 0 ? (
                <dl className="space-y-1.5 rounded-md bg-muted/45 p-2 text-2xs">
                  {additionalWindows.map((window) => (
                    <div
                      className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-0.5"
                      key={window.id}
                    >
                      <dt className="min-w-0 truncate text-muted-foreground">
                        {window.label}
                      </dt>
                      <dd className="text-right font-medium tabular-nums">
                        {window.remainingPercent}% remaining
                      </dd>
                      <dd className="col-span-2 text-muted-foreground">
                        Resets {formatUsageReset(window.resetsAt)}
                      </dd>
                    </div>
                  ))}
                </dl>
              ) : null}
              <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1 border-t border-border/50 pt-2 text-2xs text-muted-foreground">
                {usage.totals.creditBalance !== null ? (
                  <div
                    className="contents"
                    data-testid="provider-usage-credit-balance"
                  >
                    <span>Credits {usage.totals.creditBalance}</span>
                    <span aria-hidden="true">·</span>
                  </div>
                ) : null}
                {usage.totals.latestDailyTokens !== null ? (
                  <>
                    <span>
                      Daily {formatTokenCount(usage.totals.latestDailyTokens)}{" "}
                      tokens
                    </span>
                    <span aria-hidden="true">·</span>
                  </>
                ) : null}
                <span>
                  Updated{" "}
                  {new Date(usage.fetchedAt * 1000).toLocaleTimeString([], {
                    hour: "numeric",
                    minute: "2-digit",
                  })}
                </span>
              </div>
            </div>
          ) : (
            <div className="mt-3 rounded-md bg-muted/45 p-2 text-xs text-muted-foreground">
              {errorMessage ?? `Reading ${productLabel} allowance…`}
            </div>
          )}
        </section>
      </PopoverContent>
    </Popover>
  );
}

type UsageTriggerProps = React.ButtonHTMLAttributes<HTMLButtonElement> & {
  errorMessage: string | null;
  isLoading: boolean;
  productLabel: string;
  remainingPercent?: number;
  triggerLabel: string;
};

const ChromeUsageTrigger = React.forwardRef<
  HTMLButtonElement,
  UsageTriggerProps
>(function ChromeUsageTrigger(
  {
    className,
    errorMessage,
    isLoading,
    productLabel,
    remainingPercent,
    triggerLabel,
    ...triggerProps
  },
  ref,
) {
  return (
    <button
      {...triggerProps}
      aria-label={`Open AI usage details. ${triggerLabel}`}
      className={cn(
        "ml-auto flex h-[28px] min-w-0 max-w-[min(18rem,calc(100vw-9rem))] shrink items-center gap-1.5 overflow-hidden rounded-lg px-2 text-left text-sidebar-foreground transition-colors hover:bg-sidebar-accent focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-sidebar-ring",
        className,
      )}
      data-testid="sidebar-provider-usage"
      ref={ref}
      type="button"
    >
      <UsageRing isLoading={isLoading} remainingPercent={remainingPercent} />
      <span className="min-w-0 truncate whitespace-nowrap text-xs font-medium tabular-nums">
        {productLabel}{" "}
        {remainingPercent !== undefined
          ? `${remainingPercent}%`
          : errorMessage
            ? "Unavailable"
            : "Checking…"}
      </span>
    </button>
  );
});

const SettingsUsageTrigger = React.forwardRef<
  HTMLButtonElement,
  UsageTriggerProps
>(function SettingsUsageTrigger(
  {
    className,
    errorMessage,
    isLoading,
    productLabel,
    remainingPercent,
    triggerLabel,
    ...triggerProps
  },
  ref,
) {
  return (
    <button
      {...triggerProps}
      aria-label={`Open AI usage details. ${triggerLabel}`}
      className={cn(
        "mb-2 w-full rounded-lg border border-sidebar-border/70 bg-sidebar-accent/35 px-2 py-1.5 text-left text-sidebar-foreground transition-colors hover:bg-sidebar-accent/55 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-sidebar-ring group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:h-10 group-data-[collapsible=icon]:w-10 group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:p-1",
        className,
      )}
      data-testid="sidebar-provider-usage"
      ref={ref}
      type="button"
    >
      <span className="hidden group-data-[collapsible=icon]:block">
        <UsageRing isLoading={isLoading} remainingPercent={remainingPercent} />
      </span>
      <span className="block text-2xs font-medium text-sidebar-foreground/65 group-data-[collapsible=icon]:hidden">
        AI usage
      </span>
      <span className="mt-1 flex min-w-0 items-center gap-0.5 text-xs font-semibold tabular-nums group-data-[collapsible=icon]:hidden">
        <UsageRing isLoading={isLoading} remainingPercent={remainingPercent} />
        <span className="min-w-0 truncate">{productLabel}</span>
        <span className="ml-auto shrink-0">
          {remainingPercent !== undefined
            ? `${remainingPercent}%`
            : errorMessage
              ? "Unavailable"
              : "Checking…"}
        </span>
      </span>
    </button>
  );
});
