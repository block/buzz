import { BestieWakeController } from "./BestieWakeController";

/**
 * Free-drag floating Bestie bloom removed in phase 1.
 * Phase 2 mounts the autonomous wake / agent-add watcher here so it runs
 * app-wide while Bestie is enabled (footer nudge + DM RHS stay in sync).
 */
export function BestieGlobalOverlay() {
  return <BestieWakeController />;
}
