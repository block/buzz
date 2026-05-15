import { Loader2, RefreshCcw, RotateCw } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";

import { useUpdaterContext } from "./hooks/UpdaterProvider";
import type { UpdateStatus } from "./hooks/use-updater";

const indicatorButtonClass =
  "relative h-8 w-8 text-muted-foreground/80 hover:bg-muted/60 hover:text-foreground";

const iconClass = "h-4 w-4";

const variants: Record<
  "available" | "downloading" | "installing" | "ready",
  {
    Icon: typeof RefreshCcw;
    label: string;
    badgeColor: string;
    iconClass: string;
  }
> = {
  available: {
    Icon: RefreshCcw,
    label: "Update available",
    badgeColor: "bg-primary",
    iconClass: iconClass,
  },
  downloading: {
    Icon: Loader2,
    label: "Downloading update\u2026",
    badgeColor: "bg-primary",
    iconClass: `${iconClass} animate-spin`,
  },
  installing: {
    Icon: Loader2,
    label: "Installing update\u2026",
    badgeColor: "bg-primary",
    iconClass: `${iconClass} animate-spin`,
  },
  ready: {
    Icon: RotateCw,
    label: "Restart to update",
    badgeColor: "bg-emerald-500",
    iconClass: iconClass,
  },
};

function getVariant(state: UpdateStatus["state"]) {
  if (
    state === "available" ||
    state === "downloading" ||
    state === "installing" ||
    state === "ready"
  ) {
    return variants[state];
  }
  return null;
}

export function UpdateIndicator({ className }: { className?: string }) {
  const { status, downloadAndInstall, relaunch } = useUpdaterContext();
  const variant = getVariant(status.state);

  if (!variant) {
    return null;
  }

  const { Icon, label, badgeColor, iconClass: variantIconClass } = variant;
  const isActionable = status.state === "available" || status.state === "ready";
  const handleClick =
    status.state === "ready"
      ? relaunch
      : status.state === "available"
        ? downloadAndInstall
        : null;

  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label={label}
          className={`${indicatorButtonClass} ${className ?? ""}`}
          disabled={!isActionable}
          onClick={() => {
            if (handleClick) {
              void handleClick();
            }
          }}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Icon className={variantIconClass} />
          <span
            className={`absolute right-1 top-1 h-1.5 w-1.5 rounded-full ${badgeColor} animate-pulse`}
          />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}
