/** Bestie Scratch — lightweight personal notes pad (RHS Scratch). */

import type { BestieListScope } from "./bestieListTypes";

export type BestieScratchScope = BestieListScope;

export type BestieScratchNote = {
  body: string;
  createdAt: number;
  id: string;
  /** Message id that created this note (agent/NL path), when known. */
  sourceMessageId: string | null;
  title: string;
  updatedAt: number;
};

export type BestieScratchState = {
  notes: BestieScratchNote[];
  /** Message ids already applied so reloads don't duplicate agent/NL adds. */
  processedMessageIds: string[];
  version: 1;
};

export type BestieScratchAddInput = {
  body?: string;
  sourceMessageId?: string | null;
  title?: string;
};

export type BestieScratchUpdateInput = {
  body?: string;
  id: string;
  title?: string;
};
