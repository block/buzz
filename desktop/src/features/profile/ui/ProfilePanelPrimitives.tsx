import type { LucideIcon } from "lucide-react";
import { ArrowUpRight, Copy } from "lucide-react";
import type * as React from "react";
import { toast } from "sonner";

import { cn } from "@/shared/lib/cn";

const PANEL_SURFACE_CLASS = "overflow-hidden rounded-2xl bg-muted/20";
const PANEL_ICON_CLASS =
  "flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60";
const PANEL_ROW_BASE_CLASS =
  "flex w-full gap-3 px-4 text-left transition-colors";
const PANEL_ROW_INTERACTIVE_CLASS = "hover:bg-muted/40";

async function copyToClipboard(value: string, label?: string) {
  await navigator.clipboard.writeText(value);
  toast.success(label ? `Copied ${label}` : "Copied to clipboard");
}

export function ProfilePanelSurface({
  children,
  className,
  testId,
}: {
  children: React.ReactNode;
  className?: string;
  testId?: string;
}) {
  return (
    <div className={cn(PANEL_SURFACE_CLASS, className)} data-testid={testId}>
      {children}
    </div>
  );
}

export function ProfilePanelIcon({ icon: Icon }: { icon: LucideIcon }) {
  return (
    <span className={PANEL_ICON_CLASS}>
      <Icon className="h-4 w-4 text-muted-foreground" />
    </span>
  );
}

type ProfilePanelRowProps = {
  align?: "center" | "start";
  children?: React.ReactNode;
  className?: string;
  copyLabel?: string;
  copyValue?: string;
  icon: LucideIcon;
  label: React.ReactNode;
  onClick?: () => void;
  openLabel?: string;
  testId?: string;
  trailing?: React.ReactNode;
  value?: React.ReactNode;
  valueClassName?: string;
  valueTitle?: string;
};

export function ProfilePanelRow({
  align = "center",
  children,
  className,
  copyLabel,
  copyValue,
  icon,
  label,
  onClick,
  openLabel,
  testId,
  trailing,
  value,
  valueClassName,
  valueTitle,
}: ProfilePanelRowProps) {
  const isCopyable = Boolean(copyValue);
  const isActionable = Boolean(onClick);
  const actionLabel = openLabel ?? (typeof label === "string" ? label : "row");

  const defaultTrailing =
    trailing !== undefined ? (
      trailing
    ) : isActionable ? (
      <ArrowUpRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    ) : isCopyable ? (
      <Copy className="h-4 w-4 shrink-0 text-muted-foreground" />
    ) : null;

  const content = (
    <>
      <ProfilePanelIcon icon={icon} />
      <span className="min-w-0 flex-1 text-left">
        <span className="block text-xs font-medium text-foreground">
          {label}
        </span>
        {children ??
          (value !== undefined ? (
            <span
              className={cn(
                "mt-0.5 block truncate text-sm text-muted-foreground",
                valueClassName,
              )}
              title={valueTitle}
            >
              {value}
            </span>
          ) : null)}
      </span>
      {defaultTrailing}
    </>
  );

  const rowClassName = cn(
    PANEL_ROW_BASE_CLASS,
    align === "start" ? "items-start py-3" : "items-center py-3",
    className,
  );

  if (isActionable) {
    return (
      <button
        aria-label={`Open ${actionLabel}`}
        className={cn(rowClassName, PANEL_ROW_INTERACTIVE_CLASS)}
        data-testid={testId}
        onClick={onClick}
        title={`Open ${actionLabel}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  if (isCopyable && copyValue) {
    return (
      <button
        aria-label={`Copy ${actionLabel}`}
        className={cn(rowClassName, PANEL_ROW_INTERACTIVE_CLASS)}
        data-testid={testId}
        onClick={() =>
          void copyToClipboard(copyValue, copyLabel ?? actionLabel)
        }
        title={`Copy ${actionLabel}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  return (
    <div className={rowClassName} data-testid={testId}>
      {content}
    </div>
  );
}
