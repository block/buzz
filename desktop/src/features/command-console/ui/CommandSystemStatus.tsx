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
      <CardContent>
        <CardDescription>{service.detail}</CardDescription>
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
          Read-only status from the active Buzz relay, local compute, and the
          native LM Studio probe.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-2">
        {status.liveServices.map((service) => (
          <ServiceCard key={service.id} service={service} />
        ))}
      </div>

      <div className="mb-4 mt-6">
        <h3 className="text-base font-semibold">Later capabilities</h3>
        <p className="text-sm text-muted-foreground">
          These integrations are intentionally not connected yet.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {status.laterCapabilities.map((service) => (
          <ServiceCard key={service.id} service={service} />
        ))}
      </div>
    </section>
  );
}
