import { AlertTriangle } from "lucide-react";
import * as React from "react";
import type { CSSProperties, ReactNode } from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { usePersonaConversation } from "@/features/agents/usePersonaConversation";
import { useActiveAgentTurnsByChannel } from "@/features/agents/activeAgentTurnsStore";
import { useCommandDecisionActions } from "@/features/command-console/hooks/useCommandDecisionActions";

import { COMMAND_TEAM_PERSONAS } from "../domain/commandTeam";
import { useCommandConsoleStatus } from "../hooks/useCommandConsoleStatus";
import { useDailyCommandBrief } from "../hooks/useDailyCommandBrief";
import { useModelRoutingPreference } from "../hooks/useModelRoutingPreference";
import { CommandAdviserHero } from "./CommandAdviserHero";
import { CommandTeamStrip } from "./CommandTeamStrip";
import type { BriefDecisionActions } from "./BriefDecisionSection";
import { DailyCommandBrief } from "./DailyCommandBrief";
import { ModelRoutingControls } from "./ModelRoutingControls";
import { WorldMonitorConnectionCard } from "./WorldMonitorConnectionCard";

const ACTIVE_BRIEF_STATES = new Set([
  "queued",
  "collecting_sources",
  "running_specialists",
  "consolidating",
  "persisting",
]);

const COMMAND_ADVISER_THEME = {
  "--accent": "188 65% 45%",
  "--accent-foreground": "211 85% 8%",
  "--background": "211 85% 8%",
  "--border": "211 28% 27%",
  "--card": "211 67% 11%",
  "--card-foreground": "210 40% 96%",
  "--foreground": "210 40% 96%",
  "--input": "211 28% 27%",
  "--muted": "211 32% 18%",
  "--muted-foreground": "214 20% 70%",
  "--popover": "211 67% 11%",
  "--popover-foreground": "210 40% 96%",
  "--primary": "41 68% 58%",
  "--primary-foreground": "211 85% 8%",
  "--ring": "41 68% 58%",
  "--secondary": "211 40% 17%",
  "--secondary-foreground": "210 40% 96%",
} as CSSProperties;

const CHIEF_PERSONA_ID =
  COMMAND_TEAM_PERSONAS.find((persona) => persona.adviser === "chief_of_staff")
    ?.personaId ?? "builtin:command-chief-of-staff";

type CommandConsoleContentProps = {
  commandTeam?: ReactNode;
  decisionActions: BriefDecisionActions;
  systemStatus: ReturnType<typeof useCommandConsoleStatus>;
  commandBrief: ReturnType<typeof useDailyCommandBrief>;
  modelRouting: ReturnType<typeof useModelRoutingPreference>;
  activeAgentWork: boolean;
};

function CommandConsoleContent({
  commandTeam,
  decisionActions,
  systemStatus,
  commandBrief,
  modelRouting,
  activeAgentWork,
}: CommandConsoleContentProps) {
  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto bg-[#031426] text-foreground"
      data-testid="command-console-screen"
      style={COMMAND_ADVISER_THEME}
    >
      <main className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
        <CommandAdviserHero
          routingControls={
            <ModelRoutingControls
              activeWork={activeAgentWork}
              disabled={
                commandBrief.busy ||
                modelRouting.loading ||
                modelRouting.saving ||
                (commandBrief.status !== null &&
                  ACTIVE_BRIEF_STATES.has(commandBrief.status.state))
              }
              error={modelRouting.error}
              onChange={(preference) => {
                void modelRouting.setPreference(preference);
              }}
              preference={modelRouting.preference}
            />
          }
        />

        <section className="flex gap-3 rounded-xl border border-warning/30 bg-warning/10 p-4">
          <AlertTriangle
            className="mt-0.5 h-5 w-5 shrink-0 text-warning"
            aria-hidden="true"
          />
          <div>
            <h2 className="text-sm font-semibold">
              Advisory, non-accredited decision support
            </h2>
            <p className="mt-1 text-sm text-muted-foreground">
              Command Adviser supports human judgement. It does not make
              navigational decisions, issue executable orders, or control ship
              systems.
            </p>
          </div>
        </section>

        <WorldMonitorConnectionCard />

        <DailyCommandBrief
          busy={commandBrief.busy}
          decisionActions={decisionActions}
          error={commandBrief.error}
          history={commandBrief.history}
          latest={commandBrief.latest}
          loading={commandBrief.loading}
          onCancel={() => {
            void commandBrief.cancel();
          }}
          onGenerate={() => {
            void commandBrief.start();
          }}
          onScheduleChange={(update) => {
            void commandBrief.updateSchedule(update);
          }}
          schedule={commandBrief.schedule}
          status={commandBrief.status}
          systemStatus={systemStatus}
        />

        {commandTeam ?? <CommandTeamStrip />}
      </main>
    </div>
  );
}

function CommandDecisionRuntime(
  props: Omit<CommandConsoleContentProps, "decisionActions">,
) {
  const conversation = usePersonaConversation();
  const { goChannel } = useAppNavigation();
  const openChief = React.useCallback(async () => {
    const opened = await conversation.open(CHIEF_PERSONA_ID, {
      navigate: false,
    });
    if (!opened) throw new Error("Chief of Staff is unavailable.");
    return opened;
  }, [conversation.open]);
  const decisionActions = useCommandDecisionActions({
    openChief,
    navigate: goChannel,
  });
  return <CommandConsoleContent {...props} decisionActions={decisionActions} />;
}

export function CommandConsoleScreen({
  commandTeam,
  decisionActions,
}: {
  commandTeam?: ReactNode;
  decisionActions?: BriefDecisionActions;
} = {}) {
  const systemStatus = useCommandConsoleStatus();
  const commandBrief = useDailyCommandBrief();
  const modelRouting = useModelRoutingPreference();
  const activeAgentWork = useActiveAgentTurnsByChannel().length > 0;
  const contentProps = {
    activeAgentWork,
    commandTeam,
    systemStatus,
    commandBrief,
    modelRouting,
  };
  return decisionActions ? (
    <CommandConsoleContent
      {...contentProps}
      decisionActions={decisionActions}
    />
  ) : (
    <CommandDecisionRuntime {...contentProps} />
  );
}
