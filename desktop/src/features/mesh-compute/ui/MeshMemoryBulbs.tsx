import { cn } from "@/shared/lib/cn";
import type { MeshMemoryModel } from "../meshMemoryModel";

const SEGMENT_IDS = Array.from({ length: 10 }, (_, index) => `memory-${index}`);

/** Compact segmented meter for the model's approximate memory footprint. */
export function MeshMemoryBulbs({ memory }: { memory: MeshMemoryModel }) {
  return (
    <div className="mt-2" data-testid="mesh-card-memory">
      <div
        aria-label={memory.label}
        className="flex gap-1"
        role="img"
        title={memory.label}
      >
        {SEGMENT_IDS.slice(0, memory.totalSegments).map((segmentId, index) => (
          <span
            className={cn(
              "h-2 min-w-0 flex-1 rounded-full bg-muted-foreground/20",
              index < memory.usedSegments &&
                "bg-emerald-500/80 dark:bg-emerald-400/75",
            )}
            key={segmentId}
          />
        ))}
      </div>
      <p className="mt-1 text-2xs leading-snug text-muted-foreground">
        {memory.label}
      </p>
    </div>
  );
}
