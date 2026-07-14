import type * as React from "react";
import { CheckCircle2, CircleSlash } from "lucide-react";
import type { ExtensionEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

type McpServersSectionProps = {
  extensions: ExtensionEntry[];
  runtimeId: string | null;
  variant?: "compact" | "profile";
  buzzAgentSlot?: React.ReactNode;
};

export function McpServersSection({
  buzzAgentSlot,
  extensions,
  runtimeId,
  variant = "compact",
}: McpServersSectionProps) {
  const isBuzzAgent = runtimeId === "buzz-agent";

  if (!isBuzzAgent && extensions.length === 0) {
    return null;
  }

  return (
    <div
      className={cn(
        "border-t border-border/50",
        variant === "compact" ? "mt-3 pt-2" : "divide-y divide-border/50",
      )}
    >
      <p
        className={cn(
          "text-xs font-medium text-foreground",
          variant === "compact" ? "py-2" : "px-4 py-3",
        )}
      >
        MCP Servers
      </p>

      {isBuzzAgent && buzzAgentSlot ? buzzAgentSlot : null}

      {extensions.length > 0 ? (
        <div className="divide-y divide-border/50">
          {extensions.map((extension) => (
            <McpServerRow
              extension={extension}
              key={`${extension.kind}:${extension.name}`}
              variant={variant}
            />
          ))}
        </div>
      ) : (
        <p
          className={cn(
            "text-sm text-muted-foreground",
            variant === "compact" ? "py-2" : "px-4 py-3",
          )}
        >
          No custom servers configured
        </p>
      )}
    </div>
  );
}

function McpServerRow({
  extension,
  variant,
}: {
  extension: ExtensionEntry;
  variant: "compact" | "profile";
}) {
  const StatusIcon = extension.enabled ? CheckCircle2 : CircleSlash;

  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-3",
        variant === "compact" ? "py-2" : "px-4 py-3",
      )}
    >
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted/50">
        <StatusIcon
          className={cn(
            "h-4 w-4",
            extension.enabled ? "text-emerald-600" : "text-muted-foreground",
          )}
        />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {extension.name}
        </span>
        <span className="mt-0.5 block truncate text-2xs text-muted-foreground/70">
          {extension.kind}
          {extension.enabled ? " enabled" : " disabled"}
        </span>
      </span>
    </div>
  );
}
