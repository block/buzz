import { Maximize2, Minimize2, Monitor } from "lucide-react";
import * as React from "react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { cn } from "@/shared/lib/cn";
import { truncateNpub } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Dialog, DialogContent, DialogTitle } from "@/shared/ui/dialog";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { useHuddleScreen } from "../HuddleScreenContext";
import { ScreenVideoPlayer } from "./ScreenVideoPlayer";

/** Live screens occupy the call surface, not the message timeline. */
export function HuddleScreenViewer({ compact = false }: { compact?: boolean }) {
  const { screens } = useHuddleScreen();
  const [selected, setSelected] = React.useState<string | null>(null);
  const [expanded, setExpanded] = React.useState(false);
  const pubkeys = React.useMemo(
    () => screens.map((screen) => screen.pubkey),
    [screens],
  );
  const profiles = useUsersBatchQuery(pubkeys);
  React.useEffect(() => {
    if (screens.length === 0) {
      setExpanded(false);
      setSelected(null);
    }
  }, [screens.length]);
  const screen =
    screens.find((entry) => entry.pubkey === selected) ?? screens[0];
  if (!screen) return null;
  const name =
    profiles.data?.profiles[screen.pubkey]?.displayName ??
    truncateNpub(screen.pubkey);
  const image = (
    <div
      className={cn(
        "relative flex min-h-0 w-full items-center justify-center overflow-hidden bg-black",
        expanded ? "h-[min(70dvh,50rem)]" : "h-[min(35dvh,24rem)]",
        !expanded && "max-h-[max(5rem,calc(100dvh-26rem))]",
      )}
    >
      <ScreenVideoPlayer
        stream={screen}
        label={`${name}'s shared screen: ${screen.share.label}`}
      />
    </div>
  );
  const expandButton = (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          aria-label="Expand shared screen"
          onClick={() => setExpanded(true)}
          size="icon"
          variant="ghost"
          className="h-8 w-8 shrink-0"
        >
          <Maximize2 className="h-4 w-4" />
        </Button>
      </TooltipTrigger>
      <TooltipContent>Expand shared screen</TooltipContent>
    </Tooltip>
  );
  return (
    <>
      {compact ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              aria-label="View shared screen"
              onClick={() => setExpanded(true)}
              size="icon"
              variant="secondary"
              className="buzz-huddle-control-button h-12 w-12 shrink-0 rounded-md"
            >
              <Monitor className="h-4 w-4" />
            </Button>
          </TooltipTrigger>
          <TooltipContent>View shared screen</TooltipContent>
        </Tooltip>
      ) : (
        <section
          aria-label="Shared screens"
          className="shrink-0 border-b border-border"
        >
          <div className="flex min-w-0 items-center gap-2 px-3 py-1">
            <span className="min-w-0 flex-1 truncate text-sm">
              {name}: {screen.share.label}
            </span>
            {expandButton}
          </div>
          {image}
          {screens.length > 1 ? (
            <fieldset
              className="flex gap-1 overflow-x-auto px-3 py-1"
              aria-label="Choose shared screen"
            >
              {screens.map((entry) => (
                <Button
                  key={entry.pubkey}
                  size="sm"
                  variant="ghost"
                  aria-pressed={entry.pubkey === screen.pubkey}
                  onClick={() => setSelected(entry.pubkey)}
                >
                  <Monitor className="mr-2 h-4 w-4 shrink-0" />
                  {profiles.data?.profiles[entry.pubkey]?.displayName ??
                    truncateNpub(entry.pubkey)}
                </Button>
              ))}
            </fieldset>
          ) : null}
        </section>
      )}
      <Dialog open={expanded} onOpenChange={setExpanded}>
        <DialogContent
          className="max-w-[calc(100vw-2rem)] rounded-lg p-3"
          aria-describedby={undefined}
          showCloseButton={false}
        >
          <div className="flex min-w-0 items-center justify-between gap-3">
            <DialogTitle className="min-w-0 break-words text-base tracking-normal">
              {name}: {screen.share.label}
            </DialogTitle>
            <Button
              aria-label="Collapse shared screen"
              size="icon"
              variant="ghost"
              onClick={() => setExpanded(false)}
            >
              <Minimize2 className="h-4 w-4" />
            </Button>
          </div>
          {image}
        </DialogContent>
      </Dialog>
    </>
  );
}
