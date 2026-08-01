/**
 * Desktop's read-only view of the signed channel-panel contract.
 *
 * Transport adapters should validate the wire payload before constructing
 * these values. The renderer intentionally does not fetch or execute anything
 * named by a manifest.
 */

export type PanelStatus =
  | "pending"
  | "active"
  | "complete"
  | "blocked"
  | "failed"
  | "stale"
  | "unavailable";

export type PanelPresentation = "text" | "monospace" | "timestamp" | "status";

export type PanelLinkTarget =
  | "canvas"
  | "workflow"
  | "handoff"
  | "thread"
  | "event"
  | "external";

export type PanelLink = {
  label: string;
  target: PanelLinkTarget;
  sourceEventId?: string;
  uri?: string;
};

export type PanelSourceEvent = {
  eventId: string;
  kind: number;
  channelId: string;
  label: string;
};

export type PanelField = {
  label: string;
  value: string;
  presentation: PanelPresentation;
};

export type PanelSection = {
  id: string;
  title: string;
  status: PanelStatus;
  fields: PanelField[];
  links: PanelLink[];
};

export type PanelManifest = {
  schemaVersion: number;
  panelId: string;
  channelId: string;
  title: string;
  description?: string;
  status: PanelStatus;
  updatedAt: number;
  sections: PanelSection[];
  sourceEvents: PanelSourceEvent[];
};

export type SignedChannelPanelState =
  | { kind: "loading" }
  | { kind: "empty"; message?: string }
  | { kind: "ready"; manifest: PanelManifest }
  | { kind: "stale"; manifest: PanelManifest }
  | { kind: "unavailable"; message: string }
  | { kind: "invalid"; message: string };
