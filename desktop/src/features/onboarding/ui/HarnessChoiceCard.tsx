import type * as React from "react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Card, type CardProps } from "@/shared/ui/card";
import { getRuntimeDisplayLabel, RuntimeIcon } from "./RuntimeIcon";

/**
 * Setup-page width that mirrors one column of the untouched chooser grid:
 * the onboarding frame is min(viewport - page padding, 65rem), with three
 * 1rem gaps divided across four columns. Do not apply this to the chooser —
 * its grid remains the source of truth.
 */
export const HARNESS_SETUP_CARD_WIDTH_CLASS =
  "w-full max-w-[288px] md:w-[calc((min(100vw-2rem,65rem)-3rem)/4)]";

type HarnessChoiceCardProps = Omit<CardProps, "variant"> & {
  runtime: AcpRuntimeCatalogEntry;
};

/**
 * The single visual for an onboarding harness card — icon + name on the
 * textured surface. The chooser renders it interactive; the setup page
 * renders it static. Both pages keep identical size and content.
 */
export function HarnessChoiceCard({
  className,
  runtime,
  ...cardProps
}: HarnessChoiceCardProps): React.JSX.Element {
  return (
    <Card
      className={cn(
        // Height compresses on short windows (800×500 minimum) so the grid +
        // docked footer fit without clipping. The textured variant's
        // min-height floor must drop with it (see card-texture.css — compact
        // cards opt in via the variable). Chooser and setup page share this
        // class, keeping the card-travel transition dimension-stable.
        "h-[180px] w-full select-none items-center px-3 py-1.5 text-center [--buzz-card-textured-min-height:180px] [@media(max-height:560px)]:h-[120px] [@media(max-height:560px)]:[--buzz-card-textured-min-height:120px]",
        className,
      )}
      variant="textured"
      {...cardProps}
    >
      <div className="flex min-w-0 flex-col items-center gap-3">
        <RuntimeIcon className="h-8 w-8" runtime={runtime} />
        <h2 className="truncate text-sm font-normal leading-5 text-foreground">
          {getRuntimeDisplayLabel(runtime)}
        </h2>
      </div>
    </Card>
  );
}
