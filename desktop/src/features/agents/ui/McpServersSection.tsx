import type * as React from "react";
import type { ExtensionEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

type McpServersSectionProps = {
  extensions: ExtensionEntry[];
  runtimeId: string | null;
  variant?: "compact" | "profile";
  buzzAgentSlot?: React.ReactNode;
};

export function shouldRenderMcpServers(
  runtimeId: string | null,
  extensions: ExtensionEntry[],
): boolean {
  // buzz-agent always surfaces its built-in MCP servers even with no
  // user-configured extensions; every other runtime shows the section only
  // once it has extensions parsed from its config file.
  return runtimeId === "buzz-agent" || extensions.length > 0;
}

export function McpServersSection({
  buzzAgentSlot,
  extensions,
  runtimeId,
  variant = "compact",
}: McpServersSectionProps) {
  const isBuzzAgent = runtimeId === "buzz-agent";

  if (!shouldRenderMcpServers(runtimeId, extensions)) {
    return null;
  }

  return (
    <div
      className={cn(
        variant === "compact"
          ? "mt-3 border-t border-border/50 pt-2"
          : "divide-y divide-border/55",
      )}
    >
      {variant === "compact" ? (
        <p className="py-2 text-xs font-medium text-foreground">MCP servers</p>
      ) : null}

      {isBuzzAgent && buzzAgentSlot ? buzzAgentSlot : null}

      {extensions.length > 0 ? (
        <div className="divide-y divide-border/55">
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
            variant === "compact"
              ? "py-2"
              : "flex min-h-16 items-center px-4 py-3",
          )}
        >
          No custom servers configured.
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
  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-3",
        variant === "compact" ? "py-2" : "min-h-16 px-4 py-3",
      )}
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium text-foreground">
          {extension.name}
        </span>
        <span className="mt-0.5 block truncate text-sm text-muted-foreground/70">
          {extension.kind}
        </span>
      </span>
      <span className="shrink-0 text-sm text-muted-foreground">
        {extension.enabled ? "Enabled" : "Disabled"}
      </span>
    </div>
  );
}
