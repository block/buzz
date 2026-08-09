import type { AdviserId } from "@/features/command-console/domain/briefContracts";

export type ShipRoomId =
  | "dse"
  | "plans"
  | "cic"
  | "chart-house"
  | "wardroom"
  | "meeting-room"
  | "ships-office"
  | "supply-office";

export type ShipLocationId = ShipRoomId | "personnel-strip";
export type AgentLifecycle = "online" | "waking" | "offline";
export type LivingShipAgentId = AdviserId | "keeper";

export type CollaborationContext =
  | "operations"
  | "intelligence"
  | "navigation"
  | "command"
  | "planning"
  | "reporting"
  | "routine"
  | "logistics";

export type ShipRoom = {
  id: ShipRoomId;
  label: string;
  zone: "aft" | "forward";
  row: number;
  column: number;
  x: number;
  y: number;
  width: number;
  height: number;
};

export const SHIP_ROOMS = Object.freeze([
  {
    id: "dse",
    label: "DSE Operator Room",
    zone: "aft",
    row: 0,
    column: 0,
    x: 168,
    y: 447,
    width: 216,
    height: 78,
  },
  {
    id: "plans",
    label: "Plans Room",
    zone: "aft",
    row: 1,
    column: 0,
    x: 168,
    y: 539,
    width: 216,
    height: 92,
  },
  {
    id: "cic",
    label: "C.I.C.",
    zone: "forward",
    row: 0,
    column: 0,
    x: 1170,
    y: 447,
    width: 140,
    height: 82,
  },
  {
    id: "chart-house",
    label: "Chart House",
    zone: "forward",
    row: 0,
    column: 1,
    x: 1312,
    y: 447,
    width: 132,
    height: 82,
  },
  {
    id: "wardroom",
    label: "Wardroom",
    zone: "forward",
    row: 1,
    column: 0,
    x: 1170,
    y: 537,
    width: 140,
    height: 76,
  },
  {
    id: "meeting-room",
    label: "Meeting Room",
    zone: "forward",
    row: 1,
    column: 1,
    x: 1312,
    y: 537,
    width: 132,
    height: 76,
  },
  {
    id: "ships-office",
    label: "Ship's Office",
    zone: "forward",
    row: 2,
    column: 0,
    x: 1170,
    y: 605,
    width: 140,
    height: 76,
  },
  {
    id: "supply-office",
    label: "Supply Office",
    zone: "forward",
    row: 2,
    column: 1,
    x: 1312,
    y: 605,
    width: 132,
    height: 76,
  },
] as const satisfies readonly ShipRoom[]);

export type LivingShipAdviser = {
  adviser: LivingShipAgentId;
  personaId: string;
  label: string;
  shortLabel: string;
  homeRoom: ShipRoomId;
  spriteColumn: number;
};

export const LIVING_SHIP_ADVISERS = Object.freeze([
  {
    adviser: "chief_of_staff",
    personaId: "builtin:command-chief-of-staff",
    label: "Chief of Staff",
    shortLabel: "CoS",
    homeRoom: "meeting-room",
    spriteColumn: 0,
  },
  {
    adviser: "operations",
    personaId: "builtin:command-operations",
    label: "Operations",
    shortLabel: "OPS",
    homeRoom: "cic",
    spriteColumn: 1,
  },
  {
    adviser: "intelligence",
    personaId: "builtin:command-intelligence",
    label: "Maritime N2",
    shortLabel: "N2",
    homeRoom: "dse",
    spriteColumn: 2,
  },
  {
    adviser: "logistics",
    personaId: "builtin:command-logistics",
    label: "Logistics",
    shortLabel: "LOG",
    homeRoom: "supply-office",
    spriteColumn: 3,
  },
  {
    adviser: "navigation",
    personaId: "builtin:command-navigation",
    label: "Navigation",
    shortLabel: "NAV",
    homeRoom: "chart-house",
    spriteColumn: 4,
  },
  {
    adviser: "daily_routine",
    personaId: "builtin:command-daily-routine",
    label: "Daily Routine",
    shortLabel: "RTN",
    homeRoom: "ships-office",
    spriteColumn: 5,
  },
  {
    adviser: "reporting",
    personaId: "builtin:command-reporting",
    label: "Reporting",
    shortLabel: "RPT",
    homeRoom: "ships-office",
    spriteColumn: 6,
  },
  {
    adviser: "plans",
    personaId: "builtin:command-plans",
    label: "Plans",
    shortLabel: "PLAN",
    homeRoom: "plans",
    spriteColumn: 7,
  },
  {
    adviser: "keeper",
    personaId: "builtin:keeper",
    label: "Keeper",
    shortLabel: "KEEP",
    homeRoom: "ships-office",
    spriteColumn: 5,
  },
] as const satisfies readonly LivingShipAdviser[]);

const ADVISER_BY_ID = new Map(
  LIVING_SHIP_ADVISERS.map((entry) => [entry.adviser, entry] as const),
);

const COLLABORATION_ROOM: Readonly<Record<CollaborationContext, ShipRoomId>> = {
  operations: "cic",
  intelligence: "cic",
  navigation: "chart-house",
  command: "meeting-room",
  planning: "meeting-room",
  reporting: "ships-office",
  routine: "ships-office",
  logistics: "supply-office",
};

export type AgentLocationReason =
  | "unavailable"
  | "confirmed-idle"
  | "working-home"
  | "collaboration-explicit"
  | "collaboration-context";

export function resolveAgentLocation(input: {
  adviser: LivingShipAgentId;
  lifecycle: AgentLifecycle;
  working: boolean;
  collaboration?: {
    id: string;
    workspace?: ShipRoomId | null;
    context?: CollaborationContext | string | null;
  } | null;
}): { locationId: ShipLocationId; reason: AgentLocationReason } {
  if (input.lifecycle !== "online") {
    return { locationId: "personnel-strip", reason: "unavailable" };
  }

  if (!input.working) {
    return { locationId: "wardroom", reason: "confirmed-idle" };
  }

  if (input.collaboration?.workspace) {
    return {
      locationId: input.collaboration.workspace,
      reason: "collaboration-explicit",
    };
  }

  const context = input.collaboration?.context;
  if (context && context in COLLABORATION_ROOM) {
    return {
      locationId: COLLABORATION_ROOM[context as CollaborationContext],
      reason: "collaboration-context",
    };
  }

  return {
    locationId: ADVISER_BY_ID.get(input.adviser)?.homeRoom ?? "wardroom",
    reason: "working-home",
  };
}
