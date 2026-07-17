import {
  ChevronRight,
  Copy,
  FileText,
  Hash,
  MessageSquare,
  type LucideIcon,
} from "lucide-react";
import type * as React from "react";
import { toast } from "sonner";

import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Switch } from "@/shared/ui/switch";
import { writeTextToClipboard } from "@/shared/lib/clipboard";

function getChannelIcon(channelType: Channel["channelType"]): LucideIcon {
  if (channelType === "forum") {
    return FileText;
  }
  if (channelType === "dm") {
    return MessageSquare;
  }
  return Hash;
}

export function ChannelHero({ channel }: { channel: Channel }) {
  const Icon = getChannelIcon(channel.channelType);

  return (
    <div className="flex flex-col items-center gap-3 text-center">
      <div className="flex h-20 w-20 items-center justify-center rounded-full bg-primary text-primary-foreground">
        <Icon className="h-8 w-8" />
      </div>
      <div className="flex max-w-full flex-col items-center">
        <h3 className="max-w-full truncate text-xl font-semibold tracking-tight">
          {channel.name}
        </h3>
      </div>
    </div>
  );
}

export function ChannelQuickAction({
  active,
  disabled,
  icon: Icon,
  label,
  onClick,
  testId,
}: {
  active?: boolean;
  disabled?: boolean;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId?: string;
}) {
  return (
    <button
      className="flex w-16 flex-col items-center gap-2 disabled:cursor-not-allowed disabled:opacity-50"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <span
        className={cn(
          "flex h-14 w-14 items-center justify-center rounded-full transition-colors",
          active
            ? "bg-foreground text-background hover:bg-foreground/90"
            : "bg-muted/60 text-foreground hover:bg-muted/80",
        )}
      >
        <Icon className="h-5 w-5" />
      </span>
      <span
        className={cn(
          "max-w-full truncate text-xs",
          active ? "text-foreground" : "text-muted-foreground",
        )}
      >
        {label}
      </span>
    </button>
  );
}

export function FieldGroup({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-2xl bg-muted/20">{children}</div>
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

export function CopyFieldRow({
  icon: Icon,
  label,
  value,
  testId,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  testId?: string;
}) {
  async function handleCopy() {
    await writeTextToClipboard(value);
    toast.success(`Copied ${label.toLowerCase()}`);
  }

  return (
    <button
      aria-label={`Copy ${label}`}
      className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
      data-testid={testId}
      onClick={() => {
        void handleCopy();
      }}
      title={`Copy ${label}`}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-xs font-medium text-foreground">
          {label}
        </span>
        <span className="mt-0.5 block truncate font-mono text-sm text-muted-foreground">
          {value}
        </span>
      </span>
      <Copy className="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

export function InfoFieldRow({
  icon: Icon,
  label,
  value,
  testId,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  testId?: string;
}) {
  return (
    <div
      className="flex w-full items-center gap-3 px-4 py-3"
      data-testid={testId}
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-left">
        <span className="block text-xs font-medium text-foreground">
          {label}
        </span>
        <span className="mt-0.5 block truncate text-sm text-muted-foreground">
          {value}
        </span>
      </span>
    </div>
  );
}

export function NarrativeGroup({ children }: { children: React.ReactNode }) {
  return (
    <div className="overflow-hidden rounded-2xl bg-muted/20">{children}</div>
  );
}

export function NarrativeField({
  icon: Icon,
  label,
  value,
  testId,
}: {
  icon: LucideIcon;
  label: string;
  value: string;
  testId: string;
}) {
  return (
    <div
      className="flex w-full items-start gap-3 px-4 py-3"
      data-testid={testId}
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-xs font-medium text-foreground">
          {label}
        </span>
        <span className="mt-1 block whitespace-pre-wrap text-sm leading-6 text-muted-foreground">
          {value}
        </span>
      </span>
    </div>
  );
}

export function IngressRow({
  description,
  icon: Icon,
  label,
  onClick,
  testId,
  trailing,
}: {
  description?: string;
  icon: LucideIcon;
  label: string;
  onClick: () => void;
  testId: string;
  trailing?: string;
}) {
  return (
    <button
      className="flex w-full items-center gap-3 rounded-2xl bg-muted/20 px-4 py-2 text-left transition-colors hover:bg-muted/40"
      data-testid={testId}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-foreground">
          {label}
        </span>
        {description ? (
          <span className="mt-0.5 block truncate text-xs text-muted-foreground">
            {description}
          </span>
        ) : null}
      </span>
      {trailing ? (
        <span className="text-sm text-muted-foreground">{trailing}</span>
      ) : null}
      <ChevronRight className="h-4 w-4 shrink-0 text-muted-foreground" />
    </button>
  );
}

export function ToggleRow({
  checked,
  description,
  disabled,
  label,
  onCheckedChange,
  testId,
}: {
  checked: boolean;
  description: string;
  disabled?: boolean;
  label: string;
  onCheckedChange: (checked: boolean) => void;
  testId: string;
}) {
  return (
    <div className="flex items-center gap-3 px-4 py-3">
      <span className="min-w-0 flex-1">
        <span className="block text-sm font-medium text-foreground">
          {label}
        </span>
        <span className="mt-0.5 block text-xs leading-5 text-muted-foreground">
          {description}
        </span>
      </span>
      <Switch
        checked={checked}
        data-testid={testId}
        disabled={disabled}
        onCheckedChange={onCheckedChange}
      />
    </div>
  );
}
