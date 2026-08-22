import { CircleHelp, Radio } from "lucide-react";

import { FuzzyLogo } from "@/shared/ui/buzz-logo/FuzzyLogo";
import {
  getUncertainHistoryCopy,
  type UncertainHistoryCertainty,
} from "./agentSessionHistoryCertainty";

/**
 * An uncertain variant means history may exist but could not be read (no
 * `owner_p` save subscription, or archive hydration incomplete). Those are
 * deliberately distinct from `"idle"`: only `"idle"` may state that there was
 * no activity. The specific variant is carried rather than a single `"unknown"`
 * so the rendered copy can name the actual gap — see
 * `getUncertainHistoryCopy`.
 */
export type AgentSessionTranscriptEmptyState =
  | "idle"
  | "loading"
  | UncertainHistoryCertainty;

/**
 * What a transcript with nothing to render says.
 *
 * Three outcomes, and the distinction between the last two is the point: an
 * idle channel may state that no activity happened, while a channel whose
 * archive could not be read may not. All uncertain wording comes from
 * `getUncertainHistoryCopy` so this view cannot drift from the module that
 * owns it.
 */
export function AgentSessionTranscriptEmptyBody({
  emptyDescription,
  isLoading,
  state,
}: {
  emptyDescription: string;
  /** True while a turn is live or the caller is still loading. */
  isLoading: boolean;
  state: AgentSessionTranscriptEmptyState;
}) {
  if (isLoading) {
    return (
      <FuzzyLogo
        ariaLabel="Waiting for ACP activity"
        className="mx-auto text-muted-foreground"
        fuzz={false}
        loop
      />
    );
  }

  if (state !== "idle" && state !== "loading") {
    const copy = getUncertainHistoryCopy(state);
    return (
      <>
        <CircleHelp className="mx-auto h-4 w-4 text-muted-foreground" />
        <p className="mt-3 text-sm font-medium">{copy.title}</p>
        <p className="mt-1 text-sm text-muted-foreground">{copy.description}</p>
      </>
    );
  }

  return (
    <>
      <Radio className="mx-auto h-4 w-4 text-muted-foreground" />
      <p className="mt-3 text-sm font-medium">No ACP activity yet</p>
      <p className="mt-1 text-sm text-muted-foreground">{emptyDescription}</p>
    </>
  );
}
