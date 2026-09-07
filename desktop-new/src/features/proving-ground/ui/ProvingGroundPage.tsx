import {
  IconActivity,
  IconAlertTriangle,
  IconChartDots,
  IconChevronRight,
  IconCloud,
  IconMoon,
  IconRocket,
  IconSun,
} from "@tabler/icons-react";
import { useState } from "react";

import { useColorScheme } from "@/shared/theme/useColorScheme";
import { Avatar } from "@/shared/ui/Avatar";
import { Button } from "@/shared/ui/Button";
import { IconButton } from "@/shared/ui/IconButton";
import { SearchField } from "@/shared/ui/SearchField";
import { SegmentedNavigation } from "@/shared/ui/SegmentedNavigation";

import { DEPLOYS, HEALTH_TONE, INCIDENTS, REGIONS } from "../data";
import {
  HealthPill,
  KeyValue,
  Metric,
  Panel,
  Row,
  Sparkline,
  StatusPill,
} from "./parts";

/**
 * The proving ground: a fictional fleet-monitoring dashboard.
 *
 * Deliberately not Buzz. Its job is to load the design system the way a real
 * product does — many decisions adjacent to each other, at density, in both
 * modes — and to be the surface a generated theme is judged on. A specimen page
 * shows one component at a time on a backdrop the designer chose; this shows
 * forty at once, which is where a missing role or a wrong neutral step appears.
 *
 * Built only from role tokens. If something here needs a value the system does
 * not have, that is the finding.
 */

const RANGES = [
  { value: "1h", label: "1 hour" },
  { value: "24h", label: "24 hours" },
  { value: "7d", label: "7 days" },
] as const;

const SEVERITY_TONE = {
  critical: "danger",
  warning: "warning",
  info: "info",
} as const;

const DEPLOY_TONE = {
  live: "success",
  rolling: "accent",
  held: "warning",
  "rolled-back": "danger",
} as const;

const DEPLOY_LABEL = {
  live: "Live",
  rolling: "Rolling out",
  held: "Held",
  "rolled-back": "Rolled back",
} as const;

export function ProvingGroundPage() {
  const { scheme, toggle } = useColorScheme();
  const [range, setRange] = useState<string>("24h");
  const [section, setSection] = useState("overview");
  const [query, setQuery] = useState("");

  const openIncidents = INCIDENTS.length;
  const failing = REGIONS.filter((r) => r.health === "failing").length;

  return (
    <div className="dash-shell">
      <nav className="dash-nav" aria-label="Sections">
        <div className="dash-nav-group">
          <span className="dash-nav-label text-body-sm">Monitor</span>
          <NavItem
            icon={<IconActivity size={16} />}
            label="Overview"
            id="overview"
            current={section}
            onSelect={setSection}
          />
          <NavItem
            icon={<IconAlertTriangle size={16} />}
            label="Incidents"
            id="incidents"
            count={openIncidents}
            current={section}
            onSelect={setSection}
          />
          <NavItem
            icon={<IconCloud size={16} />}
            label="Regions"
            id="regions"
            count={REGIONS.length}
            current={section}
            onSelect={setSection}
          />
        </div>

        <div className="dash-nav-group">
          <span className="dash-nav-label text-body-sm">Ship</span>
          <NavItem
            icon={<IconRocket size={16} />}
            label="Deploys"
            id="deploys"
            current={section}
            onSelect={setSection}
          />
          <NavItem
            icon={<IconChartDots size={16} />}
            label="Reports"
            id="reports"
            current={section}
            onSelect={setSection}
          />
        </div>
      </nav>

      <main className="dash-main">
        <header className="dash-topbar">
          <div className="flex min-w-0 flex-col gap-1">
            <h1 className="text-title text-primary">Fleet</h1>
            <p className="text-body text-secondary">
              Five regions, {openIncidents} open incidents
              {failing > 0 ? `, ${failing} failing` : ""}.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <SearchField
              value={query}
              onValueChange={setQuery}
              placeholder="Search regions and services"
            />
            <SegmentedNavigation
              label="Time range"
              items={RANGES}
              value={range}
              onValueChange={setRange}
            />
            <IconButton
              aria-label={
                scheme === "dark" ? "Use light mode" : "Use dark mode"
              }
              onClick={toggle}
              icon={
                scheme === "dark" ? (
                  <IconSun size={16} />
                ) : (
                  <IconMoon size={16} />
                )
              }
            />
            <Button variant="primary">New check</Button>
          </div>
        </header>

        <div className="dash-metric-grid">
          <Metric
            label="Requests"
            value="39.8"
            unit="k/s"
            change="+4.2% vs yesterday"
            direction="up"
          />
          <Metric
            label="Success rate"
            value="97.4"
            unit="%"
            change="−1.9 points"
            direction="down"
          />
          <Metric
            label="p95 latency"
            value="188"
            unit="ms"
            change="+61 ms"
            direction="down"
          />
          <Metric
            label="Open incidents"
            value="4"
            change="2 acknowledged"
            direction="flat"
          />
        </div>

        <div className="dash-columns">
          <div className="flex min-w-0 flex-col gap-6">
            <Panel
              title="Regions"
              description="Throughput over the selected window."
            >
              <div className="dash-region-grid">
                {REGIONS.map((region) => (
                  <article key={region.id} className="dash-region">
                    <div className="flex items-start justify-between gap-2">
                      <div className="flex min-w-0 flex-col">
                        <span className="text-body text-primary">
                          {region.name}
                        </span>
                        <span className="text-mono-sm text-tertiary">
                          {region.code}
                        </span>
                      </div>
                      <HealthPill health={region.health} />
                    </div>
                    <Sparkline
                      values={region.history}
                      tone={HEALTH_TONE[region.health]}
                    />
                    <div className="flex flex-col gap-1">
                      <KeyValue
                        label="Requests"
                        value={
                          region.throughput === 0
                            ? "—"
                            : `${(region.throughput / 1000).toFixed(1)}k/s`
                        }
                      />
                      <KeyValue
                        label="Success"
                        value={
                          region.successRate === 0
                            ? "—"
                            : `${region.successRate.toFixed(2)}%`
                        }
                      />
                      <KeyValue
                        label="p95"
                        value={
                          region.latencyMs === 0
                            ? "—"
                            : `${region.latencyMs} ms`
                        }
                      />
                    </div>
                  </article>
                ))}
              </div>
            </Panel>

            <Panel
              title="Open incidents"
              description="Newest first."
              padded={false}
              action={
                <Button variant="quiet" size="compact">
                  View all
                </Button>
              }
            >
              {INCIDENTS.map((incident) => (
                <Row key={incident.id} interactive>
                  <StatusPill tone={SEVERITY_TONE[incident.severity]}>
                    {incident.severity}
                  </StatusPill>
                  <div className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-body text-primary">
                      {incident.title}
                    </span>
                    <span className="text-body-sm text-tertiary">
                      <span className="text-mono-sm">{incident.id}</span> ·{" "}
                      {incident.region} · {incident.openedAt}
                    </span>
                  </div>
                  {incident.assignee ? (
                    <Avatar
                      size="small"
                      alt={`Assigned to ${incident.assignee.name}`}
                      fallback={incident.assignee.initial}
                    />
                  ) : (
                    <span className="text-body-sm text-disabled">
                      Unassigned
                    </span>
                  )}
                  <IconChevronRight
                    size={16}
                    className="text-tertiary"
                    aria-hidden="true"
                  />
                </Row>
              ))}
            </Panel>
          </div>

          <div className="flex min-w-0 flex-col gap-6">
            <Panel title="Recent deploys" padded={false}>
              {DEPLOYS.map((deploy) => (
                <Row key={deploy.id}>
                  <div className="flex min-w-0 flex-1 flex-col">
                    <span className="truncate text-body text-primary">
                      {deploy.service}
                    </span>
                    <span className="text-body-sm text-tertiary">
                      <span className="text-mono-sm">{deploy.version}</span> ·{" "}
                      {deploy.author} · {deploy.at}
                    </span>
                  </div>
                  <StatusPill tone={DEPLOY_TONE[deploy.status]}>
                    {DEPLOY_LABEL[deploy.status]}
                  </StatusPill>
                </Row>
              ))}
            </Panel>

            <Panel
              title="Rollout"
              description="ingest-gateway 2026.9.4 is going out region by region."
            >
              <div className="flex flex-col gap-4">
                <div className="flex flex-col gap-2">
                  <div className="flex items-baseline justify-between">
                    <span className="text-body-sm text-secondary">
                      Progress
                    </span>
                    <span className="text-body-sm text-primary tabular-nums">
                      2 of 5 regions
                    </span>
                  </div>
                  {/* A determinate bar: inset track, accent fill. */}
                  <div
                    className="h-2 overflow-hidden rounded-pill bg-inset"
                    role="progressbar"
                    aria-valuenow={40}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-label="Rollout progress"
                  >
                    <div className="h-full w-2/5 rounded-pill bg-accent" />
                  </div>
                </div>

                <div className="flex flex-col gap-1">
                  <KeyValue label="Started" value="6 min ago" />
                  <KeyValue label="Error budget" value="98.2% remaining" />
                  <KeyValue label="Next region" value="euw1" />
                </div>

                <div className="flex flex-wrap gap-2">
                  <Button variant="primary" size="compact">
                    Continue rollout
                  </Button>
                  <Button variant="quiet" size="compact">
                    Hold
                  </Button>
                  <Button variant="ghost" size="compact" disabled>
                    Roll back
                  </Button>
                </div>
              </div>
            </Panel>
          </div>
        </div>
      </main>
    </div>
  );
}

function NavItem({
  icon,
  label,
  id,
  count,
  current,
  onSelect,
}: {
  icon: React.ReactNode;
  label: string;
  id: string;
  count?: number;
  current: string;
  onSelect: (id: string) => void;
}) {
  const isCurrent = current === id;
  return (
    <button
      type="button"
      className="dash-nav-item text-body"
      aria-current={isCurrent ? "page" : undefined}
      onClick={() => onSelect(id)}
    >
      <span aria-hidden="true" className="flex shrink-0">
        {icon}
      </span>
      {label}
      {count !== undefined ? (
        <span className="dash-nav-count text-body-sm">{count}</span>
      ) : null}
    </button>
  );
}
