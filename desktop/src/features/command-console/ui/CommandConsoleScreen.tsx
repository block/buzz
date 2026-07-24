import {
  CalendarClock,
  ClipboardList,
  Compass,
  FileChartColumn,
  ListTodo,
  ShieldCheck,
  Users,
} from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/shared/ui/card";
import { useCommandConsoleStatus } from "../hooks/useCommandConsoleStatus";
import { CommandSystemStatus } from "./CommandSystemStatus";

const ADVISERS = [
  {
    description: "Command coordination and priority synthesis placeholder.",
    icon: Users,
    name: "Chief of Staff",
  },
  {
    description: "Operational readiness and activity review placeholder.",
    icon: ClipboardList,
    name: "Operations",
  },
  {
    description: "Passage and situational planning placeholder.",
    icon: Compass,
    name: "Navigation",
  },
  {
    description: "Recurring schedule and routine review placeholder.",
    icon: CalendarClock,
    name: "Daily Routine",
  },
  {
    description: "Briefing and report preparation placeholder.",
    icon: FileChartColumn,
    name: "Reporting",
  },
  {
    description: "Forward planning and decision support placeholder.",
    icon: ListTodo,
    name: "Plans",
  },
] as const;

export function CommandConsoleScreen() {
  const systemStatus = useCommandConsoleStatus();

  return (
    <div
      className="flex min-h-0 min-w-0 flex-1 flex-col overflow-y-auto"
      data-testid="command-console-screen"
    >
      <main className="mx-auto flex w-full max-w-6xl flex-col gap-6 p-6">
        <section
          className="flex items-center gap-3 rounded-xl border border-primary/40 bg-primary px-4 py-3 text-primary-foreground shadow-sm"
          data-testid="command-console-official-banner"
        >
          <ShieldCheck className="h-6 w-6 shrink-0" aria-hidden="true" />
          <div className="min-w-0">
            <p className="text-sm font-bold tracking-widest">OFFICIAL</p>
            <p className="text-sm text-primary-foreground/80">
              Command Console information is classified OFFICIAL by default.
            </p>
          </div>
        </section>

        <header className="space-y-2">
          <p className="text-sm font-medium text-muted-foreground">
            Phase 1 foundation
          </p>
          <h1 className="text-3xl font-semibold tracking-tight">
            Command Console
          </h1>
          <p className="max-w-3xl text-base text-muted-foreground">
            Adviser execution is not connected. These roles are visible now so
            the future command workspace remains explicit without presenting
            simulated advice or readiness.
          </p>
        </header>

        <CommandSystemStatus status={systemStatus} />

        <section aria-labelledby="adviser-placeholders-heading">
          <div className="mb-4">
            <h2
              className="text-lg font-semibold"
              id="adviser-placeholders-heading"
            >
              Adviser placeholders
            </h2>
            <p className="text-sm text-muted-foreground">
              No adviser can run, retrieve data, or propose actions in this
              phase.
            </p>
          </div>

          <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-3">
            {ADVISERS.map((adviser) => {
              const Icon = adviser.icon;
              return (
                <Card key={adviser.name}>
                  <CardHeader className="gap-3 pb-3">
                    <div className="flex items-start justify-between gap-3">
                      <div className="rounded-lg bg-muted p-2 text-muted-foreground">
                        <Icon className="h-5 w-5" aria-hidden="true" />
                      </div>
                      <Badge variant="warning">Not yet operational</Badge>
                    </div>
                    <CardTitle className="text-base">{adviser.name}</CardTitle>
                  </CardHeader>
                  <CardContent>
                    <CardDescription>{adviser.description}</CardDescription>
                  </CardContent>
                </Card>
              );
            })}
          </div>
        </section>
      </main>
    </div>
  );
}
