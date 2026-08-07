import { ChevronDown, FlaskConical } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";

import { useMeshExperimentalCatalog } from "../hooks/useMeshExperimentalCatalog";
import {
  describeExperimentalEmpty,
  deriveMeshExperimentalModel,
} from "../experimentalModel";

/**
 * The experimental tier of Advanced settings: models served across machines.
 *
 * ## Why this is a separate, collapsed tier
 *
 * Every other model in the picker starts when you flip the switch. These do not.
 * mesh-llm gathers a cohort over gossip and holds a settle barrier — at least
 * two participants, membership unchanged for a dwell period — then elects a
 * coordinator deterministically by VRAM. Until that forms, a node named for a
 * collective model waits.
 *
 * That waiting is indistinguishable from a slow download, so someone who was not
 * warned would read it as a hang. The disclosure therefore sits above the list,
 * before a choice is made, rather than as a toast afterwards.
 *
 * ## Why it can be empty for good reasons
 *
 * Three different facts produce an empty list — the catalog could not be read,
 * it holds no layer packages, or this machine is large enough that nothing needs
 * splitting — and the third is good news. `deriveMeshExperimentalModel` keeps
 * them distinct so this never renders an unexplained blank.
 */
export function MeshExperimentalSection({
  disabled,
  onModelChange,
  selectedModel,
}: {
  disabled: boolean;
  onModelChange: (model: string) => void;
  selectedModel: string;
}) {
  const [open, setOpen] = React.useState(false);
  // Fetch only once opened: the first call can download the catalog, which is
  // not work to do for a section nobody expanded.
  const { catalog } = useMeshExperimentalCatalog(open);
  const model = React.useMemo(
    () => deriveMeshExperimentalModel(catalog),
    [catalog],
  );
  const selected = selectedModel.trim();

  return (
    <div className="pt-1">
      <button
        aria-expanded={open}
        className="inline-flex h-9 items-center gap-1.5 text-sm font-medium text-foreground transition-colors hover:text-foreground/80 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        data-testid="mesh-experimental-toggle"
        onClick={() => setOpen((current) => !current)}
        type="button"
      >
        <FlaskConical className="h-4 w-4 text-muted-foreground" />
        <span>Experimental</span>
        <ChevronDown
          className={cn(
            "h-4 w-4 text-muted-foreground transition-transform duration-150 ease-out",
            open && "rotate-180",
          )}
        />
      </button>

      {open ? (
        <div className="mt-2 space-y-3" data-testid="mesh-experimental-panel">
          {/*
            Stated before the list, not after a selection. The wait is the whole
            character of these models, and it looks exactly like a hang to
            anyone who was not told.
          */}
          <p className="rounded-lg bg-muted/40 px-3 py-2 text-2xs leading-relaxed text-muted-foreground">
            These models are too large for this computer alone and run across
            several machines at once.{" "}
            <span className="font-medium text-foreground">
              Sharing will not start until enough well-connected computers are
              sharing the same model
            </span>{" "}
            — it waits instead of loading. For advanced setups.
          </p>

          {model.emptyReason ? (
            <p
              className="text-2xs text-muted-foreground"
              data-testid="mesh-experimental-empty"
            >
              {describeExperimentalEmpty(model.emptyReason)}
            </p>
          ) : (
            <ul className="space-y-1" data-testid="mesh-experimental-list">
              {model.options.map((option) => {
                const isSelected = selected === option.entry.name;
                return (
                  <li key={option.entry.name}>
                    <button
                      className={cn(
                        "flex w-full min-w-0 items-center justify-between gap-3 rounded-lg px-3 py-2 text-left transition-colors",
                        isSelected
                          ? "bg-primary/10 ring-1 ring-primary/40"
                          : "hover:bg-muted/50",
                        disabled && "pointer-events-none opacity-50",
                      )}
                      data-testid="mesh-experimental-option"
                      disabled={disabled}
                      onClick={() => onModelChange(option.entry.name)}
                      type="button"
                    >
                      <span className="min-w-0 truncate text-sm">
                        {option.entry.name}
                      </span>
                      <span className="shrink-0 text-2xs tabular-nums text-muted-foreground">
                        {option.detail}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}
