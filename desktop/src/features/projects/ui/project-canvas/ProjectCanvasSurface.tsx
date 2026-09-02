import { Maximize2 } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { ProjectCanvasHost } from "./ProjectCanvasHost";
import type { ProjectCanvasBroker } from "./projectCanvasBroker";
import type { ProjectCanvasSnapshots } from "./projectCanvasProtocol";

export function ProjectCanvasSurface({
  broker,
  communityId,
  full,
  onShowFullCanvas,
  projectId,
  projectName,
  projectNames,
  snapshots,
}: {
  broker: ProjectCanvasBroker | null;
  communityId: string | null;
  full: boolean;
  onShowFullCanvas: () => void;
  projectId: string;
  projectName: string;
  projectNames: readonly string[];
  snapshots: ProjectCanvasSnapshots;
}) {
  return (
    // The canvas deliberately rejects native file drops before they reach the
    // surrounding message-composer drop target.
    // biome-ignore lint/a11y/noStaticElementInteractions: drag handlers only define an event boundary; they do not expose an interaction.
    <div
      className={cn(
        "relative flex min-h-0 flex-col overflow-hidden bg-background",
        full ? "flex-1" : "h-full w-full border-l border-border",
      )}
      data-canvas-mode={full ? "full" : "preview"}
      data-testid="project-canvas-surface"
      onDragEnter={(event) => event.stopPropagation()}
      onDragLeave={(event) => event.stopPropagation()}
      onDragOver={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
      onDrop={(event) => {
        event.preventDefault();
        event.stopPropagation();
      }}
    >
      <div className="min-h-0 flex-1">
        <ProjectCanvasHost
          broker={broker}
          communityId={communityId}
          full={full}
          projectId={projectId}
          projectName={projectName}
          projectNames={projectNames}
          snapshots={snapshots}
        />
      </div>
      {!full ? (
        <div className="absolute bottom-3 right-3 z-50">
          <Tooltip disableHoverableContent>
            <TooltipTrigger asChild>
              <Button
                aria-label="Show full Canvas"
                className="h-8 w-8 border-border/80 bg-background/95 shadow-sm backdrop-blur-sm"
                data-testid="project-canvas-show-full"
                onClick={onShowFullCanvas}
                size="icon"
                type="button"
                variant="outline"
              >
                <Maximize2 className="h-4 w-4" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Show full Canvas</TooltipContent>
          </Tooltip>
        </div>
      ) : null}
    </div>
  );
}
