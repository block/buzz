import { CircleGauge, PlugZap, Server, Unplug } from "lucide-react";

import type {
  CommandConsoleStatusViewModel,
  CommandServiceState,
  CommandServiceStatus,
} from "@/features/command-console/hooks/useCommandConsoleStatus";
import { Badge } from "@/shared/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/shared/ui/card";

function badgeVariant(state: CommandServiceState) {
  switch (state) {
    case "connected":
      return "success" as const;
    case "degraded":
      return "warning" as const;
    case "unavailable":
      return "destructive" as const;
    case "offline":
    case "not_configured":
      return "secondary" as const;
  }
}

function ServiceCard({ service }: { service: CommandServiceStatus }) {
  const Icon =
    service.state === "connected"
      ? PlugZap
      : service.state === "offline"
        ? Unplug
        : service.id === "relay"
          ? Server
          : CircleGauge;

  return (
    <Card data-testid={`command-status-${service.id}`}>
      <CardHeader className="gap-3 pb-3">
        <div className="flex items-start justify-between gap-3">
          <div className="rounded-lg bg-muted p-2 text-muted-foreground">
            <Icon className="h-5 w-5" aria-hidden="true" />
          </div>
          <Badge variant={badgeVariant(service.state)}>
            {service.statusLabel}
          </Badge>
        </div>
        <CardTitle className="text-base">{service.label}</CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <CardDescription>{service.detail}</CardDescription>
        {service.facts && service.facts.length > 0 ? (
          <dl className="space-y-2 border-t pt-3 text-sm">
            {service.facts.map((fact) => (
              <div
                className="flex items-start justify-between gap-4"
                key={`${fact.label}-${fact.value}`}
              >
                <dt className="text-muted-foreground">{fact.label}</dt>
                <dd className="break-all text-right font-medium">
                  {fact.value}
                </dd>
              </div>
            ))}
          </dl>
        ) : null}
        {service.diagnostics && service.diagnostics.length > 0 ? (
          <ul className="space-y-2 rounded-lg border border-warning/30 bg-warning/10 p-3 text-sm">
            {service.diagnostics.map((diagnostic) => (
              <li key={diagnostic}>{diagnostic}</li>
            ))}
          </ul>
        ) : null}
      </CardContent>
    </Card>
  );
}

export function CommandSystemStatus({
  status,
}: {
  status: CommandConsoleStatusViewModel;
}) {
  return (
    <section
      aria-labelledby="command-system-status-heading"
      data-testid="command-system-status"
    >
      <div className="mb-4">
        <h2
          className="text-lg font-semibold"
          id="command-system-status-heading"
        >
          System status
        </h2>
        <p className="text-sm text-muted-foreground">
          Live status from the command workspace, LM Studio, RAG, Memory, and
          Apple inputs used by the Command Adviser.
        </p>
      </div>

      {status.degradedSections.length > 0 ? (
        <p
          className="mb-4 rounded-lg border border-warning/30 bg-warning/10 p-3 text-sm"
          data-testid="command-degraded-sections"
        >
          <span className="font-medium">Degraded sections:</span>{" "}
          {status.degradedSections.join(", ")}
        </p>
      ) : null}

      <div className="grid gap-4 sm:grid-cols-2">
        {status.liveServices.map((service) => (
          <ServiceCard key={service.id} service={service} />
        ))}
      </div>
    </section>
  );
}
