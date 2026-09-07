/**
 * Fixture data for the proving ground.
 *
 * A fictional product — a fleet-monitoring dashboard — deliberately unrelated to
 * Buzz. The point is to exercise the design system at the density and variety a
 * real page has, without any of it becoming a claim about what Buzz looks like.
 * Nothing here is a product decision; it is load for the tokens to carry.
 */

export type Health = "healthy" | "degraded" | "failing" | "paused";

export type Region = {
  id: string;
  name: string;
  code: string;
  health: Health;
  /** Requests per second, for the sparkline and the headline figure. */
  throughput: number;
  /** Percentage, one decimal. */
  successRate: number;
  latencyMs: number;
  /** Twelve readings, oldest first. */
  history: number[];
};

export type Incident = {
  id: string;
  title: string;
  region: string;
  severity: "critical" | "warning" | "info";
  openedAt: string;
  assignee?: { name: string; initial: string };
  acknowledged: boolean;
};

export type Deploy = {
  id: string;
  service: string;
  version: string;
  status: "live" | "rolling" | "held" | "rolled-back";
  author: string;
  at: string;
};

export const REGIONS: Region[] = [
  {
    id: "use1",
    name: "US East",
    code: "use1",
    health: "healthy",
    throughput: 18420,
    successRate: 99.98,
    latencyMs: 42,
    history: [12, 14, 13, 16, 18, 17, 19, 21, 20, 22, 24, 23],
  },
  {
    id: "usw2",
    name: "US West",
    code: "usw2",
    health: "healthy",
    throughput: 11260,
    successRate: 99.94,
    latencyMs: 51,
    history: [9, 10, 11, 10, 12, 13, 12, 14, 13, 15, 14, 16],
  },
  {
    id: "euw1",
    name: "EU West",
    code: "euw1",
    health: "degraded",
    throughput: 8940,
    successRate: 97.21,
    latencyMs: 189,
    history: [14, 13, 15, 14, 12, 11, 9, 8, 10, 9, 7, 8],
  },
  {
    id: "apse1",
    name: "AP Southeast",
    code: "apse1",
    health: "failing",
    throughput: 1180,
    successRate: 71.44,
    latencyMs: 842,
    history: [11, 12, 10, 11, 9, 7, 5, 3, 2, 2, 1, 1],
  },
  {
    id: "sae1",
    name: "SA East",
    code: "sae1",
    health: "paused",
    throughput: 0,
    successRate: 0,
    latencyMs: 0,
    history: [6, 6, 5, 6, 4, 3, 0, 0, 0, 0, 0, 0],
  },
];

export const INCIDENTS: Incident[] = [
  {
    id: "INC-4821",
    title: "Elevated 5xx from the ingest gateway",
    region: "apse1",
    severity: "critical",
    openedAt: "14 min ago",
    assignee: { name: "Priya Raman", initial: "P" },
    acknowledged: true,
  },
  {
    id: "INC-4820",
    title: "Replica lag above threshold on the primary cluster",
    region: "euw1",
    severity: "warning",
    openedAt: "38 min ago",
    assignee: { name: "Tomas Lindqvist", initial: "T" },
    acknowledged: true,
  },
  {
    id: "INC-4819",
    title: "Certificate expires in 6 days",
    region: "usw2",
    severity: "info",
    openedAt: "2 hr ago",
    acknowledged: false,
  },
  {
    id: "INC-4817",
    title: "Queue depth climbing without a matching request increase",
    region: "euw1",
    severity: "warning",
    openedAt: "5 hr ago",
    assignee: { name: "Dani Okonkwo", initial: "D" },
    acknowledged: false,
  },
];

export const DEPLOYS: Deploy[] = [
  {
    id: "d-9931",
    service: "ingest-gateway",
    version: "2026.9.4",
    status: "rolling",
    author: "Priya Raman",
    at: "6 min ago",
  },
  {
    id: "d-9930",
    service: "index-writer",
    version: "2026.9.3",
    status: "live",
    author: "Marco Bellini",
    at: "1 hr ago",
  },
  {
    id: "d-9929",
    service: "query-planner",
    version: "2026.9.3",
    status: "held",
    author: "Dani Okonkwo",
    at: "3 hr ago",
  },
  {
    id: "d-9927",
    service: "ingest-gateway",
    version: "2026.9.2",
    status: "rolled-back",
    author: "Tomas Lindqvist",
    at: "yesterday",
  },
];

/** Maps a health state to the status family that carries its meaning. */
export const HEALTH_TONE: Record<
  Health,
  "success" | "warning" | "danger" | "neutral"
> = {
  healthy: "success",
  degraded: "warning",
  failing: "danger",
  paused: "neutral",
};

export const HEALTH_LABEL: Record<Health, string> = {
  healthy: "Healthy",
  degraded: "Degraded",
  failing: "Failing",
  paused: "Paused",
};
