import { encodeRunbookRef } from "./keys";
import {
  getRunbookFromBlob,
  loadSiteRunbooksBlob,
  removeRunbookFromBlob,
  saveSiteRunbooksBlob,
  upsertRunbookInBlob,
} from "./storage";
import { emptyRunbook, emptyRunbooksBlob } from "./serialize";
import { countPendingProcedures } from "./pendingCount";
import type { SiteRunbook, SiteRunbookRef, SiteRunbooksBlob } from "./types";

type Scope = { pubkey: string; relayUrl: string };

let scope: Scope | null = null;
let blob: SiteRunbooksBlob = emptyRunbooksBlob();
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function persist() {
  if (!scope) return;
  saveSiteRunbooksBlob(scope.pubkey, scope.relayUrl, blob);
}

export function subscribeSiteRunbooks(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function configureSiteRunbooksScope(
  pubkey: string,
  relayUrl: string,
): void {
  const next = { pubkey, relayUrl };
  if (
    scope &&
    scope.pubkey === next.pubkey &&
    scope.relayUrl === next.relayUrl
  ) {
    return;
  }
  scope = next;
  blob = loadSiteRunbooksBlob(pubkey, relayUrl);
  emit();
}

export function resetSiteRunbooksStore(): void {
  scope = null;
  blob = emptyRunbooksBlob();
  emit();
}

export function getSiteRunbook(ref: SiteRunbookRef): SiteRunbook | null {
  return getRunbookFromBlob(blob, encodeRunbookRef(ref));
}

export function getSiteRunbookOrEmpty(ref: SiteRunbookRef): SiteRunbook {
  return getSiteRunbook(ref) ?? emptyRunbook();
}

export function setSiteRunbook(
  ref: SiteRunbookRef,
  runbook: SiteRunbook,
): SiteRunbook {
  blob = upsertRunbookInBlob(blob, encodeRunbookRef(ref), runbook);
  persist();
  emit();
  return runbook;
}

export function clearSiteRunbook(ref: SiteRunbookRef): void {
  const key = encodeRunbookRef(ref);
  if (!(key in blob.runbooks)) return;
  blob = removeRunbookFromBlob(blob, key);
  persist();
  emit();
}

export function listSiteRunbookKeys(): string[] {
  return Object.keys(blob.runbooks);
}

/** Live unread-style count: pending procedures across all runbooks. */
export function getPendingProcedureCount(): number {
  return countPendingProcedures(blob);
}

export function getSiteRunbooksBlob(): SiteRunbooksBlob {
  return blob;
}

/** Test helper: replace in-memory blob without touching storage. */
export function __setSiteRunbooksBlobForTests(next: SiteRunbooksBlob): void {
  blob = next;
  emit();
}

export function __getSiteRunbooksBlobForTests(): SiteRunbooksBlob {
  return blob;
}
