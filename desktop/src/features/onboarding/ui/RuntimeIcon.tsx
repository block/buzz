import * as React from "react";
import { TerminalSquare } from "lucide-react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { BuzzMark } from "@/shared/ui/buzz-logo/BuzzMark";
import claudeLogoUrl from "../assets/harness-logos/claude.png?inline";
import { RUNTIME_MARKS } from "./HarnessMarks";

// Bundled logos for compiled-in runtimes (inline base64, no network fetch).
// Monochrome marks live in RUNTIME_MARKS instead — inline SVGs that follow
// `currentColor`, so they adapt to dark/light without bitmap filters.
const RUNTIME_LOGOS: Record<string, string> = {
  claude: claudeLogoUrl,
};

// Public-path logos for bundled presets. Served from /harness-logos/ at runtime.
// Keys match the preset `id` values emitted by the backend PRESET_HARNESSES.
export const PRESET_LOGOS: Record<string, string> = {
  omp: "/harness-logos/omp.svg",
  grok: "/harness-logos/grok.svg",
  opencode: "/harness-logos/opencode.svg",
  kimi: "/harness-logos/kimi.png",
  amp: "/harness-logos/amp.png",
  hermes: "/harness-logos/hermes.png",
  openclaw: "/harness-logos/openclaw.svg",
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
  const id = runtime.id.trim().toLowerCase();
  return RUNTIME_LOGOS[id] ?? PRESET_LOGOS[id] ?? null;
}

export function RuntimeIcon({
  className = "h-8 w-8",
  runtime,
}: {
  className?: string;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const [imageFailed, setImageFailed] = React.useState(false);
  // Only use bundled logo maps — never render user-supplied avatar URLs for
  // custom/preset entries (tracking pixel / spoofing vector, security line).
  const id = runtime.id.trim().toLowerCase();
  const imageUrl = getRuntimeLogoUrl(runtime);
  const Mark = RUNTIME_MARKS[id];

  if (isBuzzRuntime(runtime)) {
    // The mark's wide viewBox letterboxes inside a square box, so honoring
    // the caller's size keeps it optically in line with the square logos.
    return <BuzzMark className={cn(className, "text-foreground")} />;
  }

  if (Mark) {
    return <Mark className={cn(className, "p-0.5 text-foreground")} />;
  }

  if (imageUrl && !imageFailed) {
    return (
      <img
        alt=""
        className={cn(
          "rounded-md object-contain",
          className,
          id === "omp" && "bg-[#0d0d0d] p-1",
          id === "grok" && "bg-white p-1",
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
