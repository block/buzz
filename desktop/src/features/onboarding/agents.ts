import agent1 from "@/assets/onboarding/agents/buzz-agent-1.png";
import agent2 from "@/assets/onboarding/agents/buzz-agent-2.png";
import agent3 from "@/assets/onboarding/agents/buzz-agent-3.png";
import agent4 from "@/assets/onboarding/agents/buzz-agent-4.png";
import agent5 from "@/assets/onboarding/agents/buzz-agent-5.png";

/**
 * Curated onboarding agents (step 3).
 *
 * PLACEHOLDER DATA — these five entries are stand-ins so the agent-selection
 * step can be built and reviewed. Replace with the real curated set (or a
 * server-driven source) later; the UI reads only from this constant so the
 * source can change without touching the step.
 *
 * `id` is a stable slug used as the selection key and (later) to resolve the
 * agent that starts the step-5 first-chat DM.
 */
export type OnboardingAgent = {
  id: string;
  name: string;
  description: string;
  avatarUrl: string;
  tags: string[];
  recommended?: boolean;
};

export const ONBOARDING_AGENTS: readonly OnboardingAgent[] = [
  {
    id: "goose",
    name: "Goose",
    description:
      "Block's open-source coding agent. Pairs with you on real work across your repos.",
    avatarUrl: agent1,
    tags: ["Coding"],
    recommended: true,
  },
  {
    id: "scout",
    name: "Scout",
    description:
      "A research assistant that digs through docs and threads to answer questions.",
    avatarUrl: agent2,
    tags: ["Research"],
  },
  {
    id: "muse",
    name: "Muse",
    description:
      "A brainstorming partner for naming, copy, and early product ideas.",
    avatarUrl: agent3,
    tags: ["Creative"],
  },
  {
    id: "tempo",
    name: "Tempo",
    description:
      "Keeps projects moving — summarizes standups and nudges on next steps.",
    avatarUrl: agent4,
    tags: ["Productivity"],
  },
  {
    id: "sage",
    name: "Sage",
    description:
      "A general-purpose assistant for everyday questions and quick tasks.",
    avatarUrl: agent5,
    tags: ["General"],
  },
] as const;
