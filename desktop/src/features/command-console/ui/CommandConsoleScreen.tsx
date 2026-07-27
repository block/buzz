import { AlertTriangle } from "lucide-react";

import { useCommandConsoleStatus } from "../hooks/useCommandConsoleStatus";
import { useDailyCommandBrief } from "../hooks/useDailyCommandBrief";
import { useModelRoutingPreference } from "../hooks/useModelRoutingPreference";
import { CommandAdviserHero } from "./CommandAdviserHero";
import { CommandSystemStatus } from "./CommandSystemStatus";
import { CommandTeamStrip } from "./CommandTeamStrip";
import { DailyCommandBrief } from "./DailyCommandBrief";
import { ModelRoutingControls } from "./ModelRoutingControls";

const ACTIVE_BRIEF_STATES = new Set([
  "queued",
  "collecting_sources",
  "running_specialists",
  "consolidating",
  "persisting",
]);

export function CommandConsoleScreen() {
  const systemStatus = useCommandConsoleStatus();
  const commandBrief = useDailyCommandBrief();
  const modelRouting = useModelRoutingPreference();

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto"
      data-testid="command-console-screen"
    >
      <main className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
        <CommandAdviserHero
          routingControls={
            <ModelRoutingControls
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

        <CommandTeamStrip />

        <CommandSystemStatus status={systemStatus} />

        <DailyCommandBrief
          busy={commandBrief.busy}
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
        />
      </main>
    </div>
  );
}
