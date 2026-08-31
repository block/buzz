import {
  Check,
  ChevronRight,
  Copy,
  Info,
  MessageSquare,
  Pencil,
  type LucideIcon,
} from "lucide-react";
import * as React from "react";
import { toast } from "sonner";

import { ChannelGlyph } from "@/features/channels/ui/ChannelGlyph";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { writeTextToClipboard } from "@/shared/lib/clipboard";
import { PanelSectionGroup } from "@/shared/ui/PanelSectionGroup";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

export function ChannelHero({
  channel,
  onEdit,
}: {
  channel: Channel;
  onEdit?: () => void;
}) {
  const channelDescription = channel.description.trim();
  const description =
    channelDescription || (onEdit ? "Add a description" : null);

  return (
    <div
      className="flex flex-col items-center gap-3 py-3 text-center"
      data-testid="channel-management-hero"
    >
      <div className="flex h-20 w-20 items-center justify-center rounded-full bg-primary text-primary-foreground">
        {channel.channelType === "dm" ? (
          <MessageSquare className="h-8 w-8" />
        ) : (
          <ChannelGlyph channel={channel} className="h-8 w-8" />
        )}
      </div>
      {channel.channelType !== "dm" && onEdit ? (
        <button
          aria-label="Edit channel"
          className="group flex max-w-full flex-col items-center rounded-lg px-8 py-1 text-center focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
          data-testid="channel-management-edit"
          onClick={onEdit}
          type="button"
        >
          <span
            className="relative max-w-full"
            data-testid="channel-management-name-row"
          >
            <span className="block max-w-full truncate text-xl font-semibold tracking-tight">
              {channel.name}
            </span>
            <span
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 left-full ml-1 -translate-y-1/2 text-muted-foreground opacity-0 transition-[color,opacity] duration-150 ease-out group-hover:text-foreground group-hover:opacity-100 group-focus-visible:opacity-100"
              data-testid="channel-management-edit-icon"
            >
              <Pencil className="h-4 w-4" />
            </span>
          </span>
          <span
            className="mt-1 line-clamp-6 max-w-full whitespace-pre-line text-sm leading-5 text-muted-foreground/70"
            data-testid="channel-management-description"
          >
            {description}
          </span>
        </button>
      ) : channel.channelType !== "dm" ? (
        <div className="flex max-w-full flex-col items-center">
          <h3
            className="max-w-full truncate text-xl font-semibold tracking-tight"
            data-testid="channel-management-name-row"
          >
            {channel.name}
          </h3>
          {description ? (
            <p
              className="mt-1 line-clamp-6 max-w-full whitespace-pre-line text-sm leading-5 text-muted-foreground/70"
              data-testid="channel-management-description"
            >
              {description}
            </p>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

export function FieldGroup({
  children,
  description,
  testId,
  title,
}: {
  children: React.ReactNode;
  description?: React.ReactNode;
  testId?: string;
  title?: React.ReactNode;
}) {
  return (
    <PanelSectionGroup description={description} testId={testId} title={title}>
      {children}
    </PanelSectionGroup>
  );
}

export function getMarkdownPreviewText(content: string) {
  return content
    .split("\n")
    .map((line) =>
      line
        .trim()
        .replace(/^#{1,6}\s+/, "")
        .replace(/^>\s?/, "")
        .replace(/^[-*+]\s+\[[ xX]\]\s+/, "")
        .replace(/^[-*+]\s+/, "")
        .replace(/^\d+\.\s+/, "")
        .replace(/!\[([^\]]*)\]\([^)]+\)/g, "$1")
        .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1")
        .replace(/`([^`]+)`/g, "$1")
        .replace(/(\*\*|__)(.*?)\1/g, "$2")
        .replace(/(\*|_)(.*?)\1/g, "$2")
        .replace(/~~(.*?)~~/g, "$1")
        .trim(),
    )
    .filter(Boolean)
    .join(" ");
}

function truncateIdentifier(value: string) {
  if (value.length <= 12) return value;
  return `${value.slice(0, 8)}…${value.slice(-4)}`;
}

export function CopyFieldRow({
  icon: Icon,
  label,
  value,
  testId,
}: {
  icon?: LucideIcon;
  label: string;
  value: string;
  testId?: string;
}) {
  const [copied, setCopied] = React.useState(false);
  const resetTimerRef = React.useRef<number | null>(null);

  React.useEffect(
    () => () => {
      if (resetTimerRef.current !== null) {
        window.clearTimeout(resetTimerRef.current);
      }
    },
    [],
  );

  async function handleCopy() {
    await writeTextToClipboard(value);
    setCopied(true);
    if (resetTimerRef.current !== null) {
      window.clearTimeout(resetTimerRef.current);
    }
    resetTimerRef.current = window.setTimeout(() => {
      setCopied(false);
      resetTimerRef.current = null;
    }, 1_500);
    toast.success(`Copied ${label.toLowerCase()}`);
  }

  return (
    <button
      aria-label={copied ? `${label} copied` : `Copy ${label}`}
      className="group flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
      data-testid={testId}
      onClick={() => {
        void handleCopy();
      }}
      title={`Copy ${label}`}
      type="button"
    >
      {Icon ? (
        <Icon
          className="h-4 w-4 shrink-0 text-muted-foreground"
          data-slot="field-row-icon"
        />
      ) : null}
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-foreground">
          {label}
        </span>
        <span
          className="mt-0.5 block truncate font-mono text-sm text-muted-foreground/70"
          title={value}
        >
          {truncateIdentifier(value)}
        </span>
      </span>
      <span
        aria-hidden="true"
        className={cn(
          "relative h-4 w-4 shrink-0 transition-opacity duration-150 ease-out motion-reduce:transition-none group-hover:opacity-100 group-focus-within:opacity-100",
          copied ? "opacity-100" : "opacity-0",
        )}
        data-copied={copied}
        data-testid={testId ? `${testId}-copy-status` : undefined}
      >
        <Copy
          className={cn(
            "absolute inset-0 h-4 w-4 transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
            copied ? "scale-95 opacity-0" : "scale-100 opacity-100",
          )}
        />
        <Check
          className={cn(
            "absolute inset-0 h-4 w-4 text-primary transition-[opacity,transform] duration-150 ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none",
            copied ? "scale-100 opacity-100" : "scale-95 opacity-0",
          )}
        />
      </span>
    </button>
  );
}

export function InfoFieldRow({
  icon: Icon,
  label,
  multiline = false,
  onClick,
  trailing,
  value,
  testId,
}: {
  icon?: LucideIcon;
  label: string;
  multiline?: boolean;
  onClick?: () => void;
  trailing?: React.ReactNode;
  value: string;
  testId?: string;
}) {
  const content = (
    <>
      {Icon ? (
        <Icon
          className="h-4 w-4 shrink-0 text-muted-foreground"
          data-slot="field-row-icon"
        />
      ) : null}
      <span className="min-w-0 flex-1 text-left">
        <span className="block text-sm font-medium text-foreground">
          {label}
        </span>
        <span
          className={cn(
            "mt-0.5 block text-sm font-normal text-muted-foreground/70",
            multiline ? "line-clamp-2 whitespace-normal leading-5" : "truncate",
          )}
        >
          {value}
        </span>
      </span>
      {trailing}
    </>
  );

  if (onClick) {
    return (
      <button
        className="flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        data-testid={testId}
        onClick={onClick}
        type="button"
      >
        {content}
      </button>
    );
  }

  return (
    <div
      className="flex min-h-16 w-full items-center gap-3 px-4 py-3"
      data-testid={testId}
    >
      {content}
    </div>
  );
}

export function EditableInfoFieldRow({
  editTestId,
  icon: Icon,
  label,
  multiline = false,
  onEdit,
  value,
  testId,
}: {
  editTestId?: string;
  icon?: LucideIcon;
  label: string;
  multiline?: boolean;
  onEdit?: () => void;
  value: string;
  testId: string;
}) {
  const content = (
    <>
      {Icon ? (
        <Icon
          className="h-4 w-4 shrink-0 text-muted-foreground"
          data-slot="field-row-icon"
        />
      ) : null}
      <span className="min-w-0 flex-1 text-left">
        <span className="block text-sm font-medium text-foreground">
          {label}
        </span>
        <span
          className={cn(
            "mt-0.5 block text-sm font-normal text-muted-foreground/70",
            multiline ? "line-clamp-2 whitespace-normal leading-5" : "truncate",
          )}
        >
          {value}
        </span>
      </span>
      {onEdit ? (
        <span
          aria-hidden="true"
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
          data-testid={editTestId}
        >
          <Pencil className="h-4 w-4" />
        </span>
      ) : null}
    </>
  );

  if (onEdit) {
    return (
      <button
        aria-label={`Edit ${label.toLowerCase()}`}
        className="group flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        data-testid={testId}
        onClick={onEdit}
        type="button"
      >
        {content}
      </button>
    );
  }

  return (
    <div
      className="group flex min-h-16 w-full items-center gap-3 px-4 py-3"
      data-testid={testId}
    >
      {content}
    </div>
  );
}

type ActionFieldRowProps = {
  destructive?: boolean;
  description?: string;
  disabled?: boolean;
  icon: LucideIcon;
  label: string;
  onClick?: () => void;
  testId: string;
};

export const ActionFieldRow = React.forwardRef<
  HTMLButtonElement,
  ActionFieldRowProps
>(function ActionFieldRow(
  {
    destructive = false,
    description,
    disabled,
    icon: Icon,
    label,
    onClick,
    testId,
    ...triggerProps
  },
  ref,
) {
  return (
    <button
      className={cn(
        "flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 disabled:cursor-not-allowed disabled:opacity-50",
        destructive && "text-destructive",
      )}
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      ref={ref}
      type="button"
      {...triggerProps}
    >
      <Icon
        className={cn(
          "h-4 w-4 shrink-0 text-muted-foreground",
          destructive && "text-destructive",
        )}
        data-slot="field-row-icon"
      />
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium">{label}</span>
        {description ? (
          <span className="mt-0.5 block truncate text-sm font-normal text-muted-foreground/70">
            {description}
          </span>
        ) : null}
      </span>
    </button>
  );
});

export function IngressRow({
  description,
  helpText,
  icon: Icon,
  label,
  onClick,
  testId,
  trailing,
}: {
  description?: string;
  helpText?: string;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId: string;
  trailing?: string;
}) {
  return (
    <PanelSectionGroup testId={`${testId}-section`}>
      <div className="relative flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left">
        <button
          aria-label={`Open ${label.toLowerCase()}`}
          className="absolute inset-0 z-10 transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          data-testid={testId}
          onClick={onClick}
          type="button"
        />
        <Icon className="pointer-events-none relative z-20 h-4 w-4 shrink-0 text-muted-foreground" />
        <span className="pointer-events-none relative z-20 min-w-0 flex-1">
          <span className="flex items-center gap-1.5 text-sm font-medium text-foreground">
            <span>{label}</span>
            {helpText ? (
              <Tooltip>
                <TooltipTrigger asChild>
                  <button
                    aria-label={`About ${label.toLowerCase()}`}
                    className="pointer-events-auto flex h-5 w-5 shrink-0 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-muted/70 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                    data-testid={`${testId}-info`}
                    onClick={(event) => event.stopPropagation()}
                    type="button"
                  >
                    <Info className="h-3.5 w-3.5" />
                  </button>
                </TooltipTrigger>
                <TooltipContent className="max-w-64 text-left" side="top">
                  {helpText}
                </TooltipContent>
              </Tooltip>
            ) : null}
          </span>
          {description ? (
            <span className="mt-0.5 block truncate text-sm font-normal text-muted-foreground/70">
              {description}
            </span>
          ) : null}
        </span>
        {trailing ? (
          <span className="pointer-events-none relative z-20 text-sm text-muted-foreground/70">
            {trailing}
          </span>
        ) : null}
        <ChevronRight className="pointer-events-none relative z-20 h-4 w-4 shrink-0 text-muted-foreground" />
      </div>
    </PanelSectionGroup>
  );
}
