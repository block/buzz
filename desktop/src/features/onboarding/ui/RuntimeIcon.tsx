import * as React from "react";
import { TerminalSquare } from "lucide-react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { useTheme } from "@/shared/theme/ThemeProvider";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import chatgptLogoUrl from "../assets/harness-logos/chatgpt.png?inline";
import claudeLogoUrl from "../assets/harness-logos/claude.png?inline";
import gooseLogoUrl from "../assets/harness-logos/goose.png?inline";

const RUNTIME_LOGOS: Record<string, string> = {
  claude: claudeLogoUrl,
  codex: chatgptLogoUrl,
  goose: gooseLogoUrl,
};

function isBuzzRuntime(runtime: AcpRuntimeCatalogEntry): boolean {
  return runtime.id.trim().toLowerCase() === "buzz-agent";
}

export function getRuntimeDisplayLabel(
  runtime: AcpRuntimeCatalogEntry,
): string {
  return isBuzzRuntime(runtime) ? "Buzz" : runtime.label;
}

function getRuntimeLogoUrl(runtime: AcpRuntimeCatalogEntry): string | null {
  return RUNTIME_LOGOS[runtime.id.trim().toLowerCase()] ?? null;
}

export function RuntimeIcon({
  className = "h-8 w-8",
  runtime,
}: {
  className?: string;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const [imageFailed, setImageFailed] = React.useState(false);
  const { isDark } = useTheme();
  const runtimeLogoUrl = getRuntimeLogoUrl(runtime);
  const imageUrl = runtimeLogoUrl ?? runtime.avatarUrl;
  const shouldForceForegroundColor = !runtimeLogoUrl && runtime.id === "goose";

  if (isBuzzRuntime(runtime)) {
    return <BuzzMark className="h-7 w-10 text-foreground" />;
  }

  if (imageUrl && !imageFailed) {
    return (
      <img
        alt=""
        className={cn(
          "rounded-md object-contain",
          className,
          shouldForceForegroundColor &&
            (isDark ? "brightness-0 invert" : "brightness-0"),
        )}
        onError={() => setImageFailed(true)}
        src={imageUrl}
      />
    );
  }

  return (
    <TerminalSquare
      className={cn(className, "text-foreground")}
      strokeWidth={1.25}
    />
  );
}
