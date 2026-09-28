/** Bestie Coffee — scheduled /hula-coffee briefing entries (RHS Coffee). */

import type { BestieListScope } from "./bestieListTypes";

export type BestieCoffeeScope = BestieListScope;

export type BestieCoffeeRunSource = "scheduled" | "brew";

export type BestieCoffeeEntry = {
  brief: string;
  fullOutput: string;
  id: string;
  /** Unix seconds when the briefing completed. */
  ranAt: number;
  replyMessageId: string | null;
  source: BestieCoffeeRunSource;
  triggerMessageId: string | null;
};

/**
 * Local-time schedule for the automatic morning coffee.
 * Default 08:00 in the machine local timezone (America/Phoenix for JM).
 * Prefs are stored so a future settings UI can adjust without schema churn.
 */
export type BestieCoffeePrefs = {
  hour: number;
  minute: number;
};

export type BestieCoffeePendingRun = {
  source: BestieCoffeeRunSource;
  startedAt: number;
  triggerMessageId: string | null;
};

export type BestieCoffeeState = {
  entries: BestieCoffeeEntry[];
  /**
   * Coffee trigger message ids the user removed from the Coffee tab.
   * Prevents WakeController / Path C from rehydrating deleted brews from chat.
   */
  forgottenTriggerIds: string[];
  /** Local YYYY-MM-DD claimed for the scheduled morning run (set at begin; not Brew). */
  lastScheduledDayKey: string | null;
  pendingRun: BestieCoffeePendingRun | null;
  prefs: BestieCoffeePrefs;
  version: 1;
};

export type BestieCoffeeAddEntryInput = {
  brief: string;
  fullOutput: string;
  replyMessageId?: string | null;
  source: BestieCoffeeRunSource;
  triggerMessageId?: string | null;
};
