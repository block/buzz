import { Check } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { ONBOARDING_AGENTS, type OnboardingAgent } from "../agents";
import {
  type OnboardingTransitionDirection,
  OnboardingSlideTransition,
} from "./OnboardingSlideTransition";
import type { AgentStepActions } from "./types";

/**
 * Onboarding agent selection (step 3).
 *
 * A centered 2-column grid of curated agent cards. Single-select (per product
 * decision — the mock's checkmark could imply multi-select, but onboarding
 * picks exactly one agent to drive the step-5 first chat). The chosen agent id
 * is owned by the parent flow; Continue is disabled until one is selected.
 */
export function AgentStep({
  actions,
  direction,
  selectedAgentId,
  onSelectAgent,
}: {
  actions: AgentStepActions;
  direction: OnboardingTransitionDirection;
  selectedAgentId: string | null;
  onSelectAgent: (id: string) => void;
}) {
  return (
    <OnboardingSlideTransition
      className="flex w-full flex-col items-center text-center"
      direction={direction}
      transitionKey={`agent-${direction}`}
    >
      <div className="flex w-full max-w-[720px] flex-col items-center">
        <h1 className="text-3xl font-semibold tracking-tight">
          Choose your agent
        </h1>
        <p className="mt-3 text-base leading-6 text-muted-foreground">
          Agents are AI teammates you can chat with. Pick one to get started —
          you can add more later.
        </p>

        <div
          role="radiogroup"
          aria-label="Choose your agent"
          className="mt-8 grid w-full grid-cols-1 gap-4 sm:grid-cols-2"
        >
          {ONBOARDING_AGENTS.map((agent) => (
            <AgentCard
              key={agent.id}
              agent={agent}
              selected={agent.id === selectedAgentId}
              onSelect={() => onSelectAgent(agent.id)}
            />
          ))}
        </div>

        <div className="mt-8 flex w-full items-center justify-between">
          <Button
            variant="ghost"
            data-testid="onboarding-back"
            onClick={actions.back}
          >
            Back
          </Button>
          <Button
            data-testid="onboarding-next"
            disabled={selectedAgentId === null}
            onClick={actions.submit}
          >
            Continue
          </Button>
        </div>
      </div>
    </OnboardingSlideTransition>
  );
}

function AgentCard({
  agent,
  selected,
  onSelect,
}: {
  agent: OnboardingAgent;
  selected: boolean;
  onSelect: () => void;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: card carries rich content (avatar, badges, tags) that can't live inside a native <input type="radio">; wrapped in a role="radiogroup" for correct semantics
    <button
      type="button"
      role="radio"
      aria-checked={selected}
      data-testid={`onboarding-agent-${agent.id}`}
      onClick={onSelect}
      className={cn(
        "group relative flex min-h-[7.5rem] flex-col rounded-xl border bg-card p-4 text-left transition-all",
        "hover:border-primary/50 hover:bg-muted/40 hover:shadow-md",
        selected ? "border-2 border-primary bg-primary/5" : "border-border/60",
      )}
    >
      {selected ? (
        <span className="absolute right-3 top-3 flex h-5 w-5 items-center justify-center rounded-full bg-primary text-primary-foreground">
          <Check className="h-3 w-3" strokeWidth={3} />
        </span>
      ) : null}

      <div className="flex items-start gap-3">
        <img
          src={agent.avatarUrl}
          alt=""
          className="h-11 w-11 shrink-0 rounded-lg object-cover"
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate font-semibold">{agent.name}</span>
            {agent.recommended ? (
              <span className="shrink-0 rounded-full bg-primary/15 px-2 py-0.5 text-2xs font-medium text-primary">
                Recommended
              </span>
            ) : null}
          </div>
          <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
            {agent.description}
          </p>
        </div>
      </div>

      <div className="mt-3 flex flex-wrap gap-1.5">
        {agent.tags.map((tag) => (
          <span
            key={tag}
            className="rounded-full bg-muted px-2 py-0.5 text-2xs font-medium text-muted-foreground"
          >
            {tag}
          </span>
        ))}
      </div>
    </button>
  );
}
