import {
  deriveCommunityName,
  normalizeRelayUrl,
} from "@/features/communities/communityStorage";
import { setLocalStorageItemWithRecovery } from "@/shared/lib/localStorageQuota";

const STORAGE_KEY = "buzz-community-onboarding-transaction.v1";

export type CommunityOnboardingSource =
  | "first-community"
  | "add-community"
  | "membership-recovery"
  | "deep-link-connect"
  | "deep-link-join";

export type CommunityOnboardingStage =
  | "claiming"
  | "connecting"
  | "profile"
  | "team-intro"
  | "finalizing";

export type CommunityOnboardingTransaction = {
  id: string;
  source: CommunityOnboardingSource;
  stage: CommunityOnboardingStage;
  relayUrl: string;
  inviteCode?: string;
  communityName: string;
  token?: string;
  reposDir?: string;
  /**
   * Join-policy acceptance receipt minted before the claim (bound to the
   * invite code). Forwarded to `claimInvite` so relays with a configured
   * join policy admit the claim.
   */
  policyReceipt?: string;
  communityId?: string;
  createdAt: string;
  updatedAt: string;
  error?: string;
  // Deep links are persisted before machine onboarding completes. Set when
  // the user dismisses the acknowledgment so it stays dismissed on relaunch.
  acknowledged?: boolean;
};

export type CommunityOnboardingTransactionPatch = Partial<
  Pick<
    CommunityOnboardingTransaction,
    "stage" | "communityId" | "communityName" | "error" | "acknowledged"
  >
>;

export type StartCommunityOnboardingInput = {
  source: CommunityOnboardingSource;
  relayUrl: string;
  inviteCode?: string;
  communityName?: string;
  token?: string;
  reposDir?: string;
  policyReceipt?: string;
};

function canonicalRelayUrl(rawRelayUrl: string) {
  const trimmed = rawRelayUrl.trim();
  const withScheme = /^(ws|wss):\/\//i.test(trimmed)
    ? trimmed
    : normalizeRelayUrl(trimmed);
  const parsed = new URL(withScheme);
  parsed.protocol = parsed.protocol.toLowerCase();
  parsed.hostname = parsed.hostname.toLowerCase();
  parsed.pathname = parsed.pathname.replace(/\/+$/, "") || "/";
  return parsed.toString().replace(/\/$/, "");
}

function isTransaction(
  value: unknown,
): value is CommunityOnboardingTransaction {
  if (!value || typeof value !== "object") return false;
  const transaction = value as Partial<CommunityOnboardingTransaction>;
  return (
    typeof transaction.id === "string" &&
    typeof transaction.relayUrl === "string" &&
    typeof transaction.communityName === "string" &&
    typeof transaction.createdAt === "string" &&
    typeof transaction.updatedAt === "string" &&
    ["claiming", "connecting", "profile", "team-intro", "finalizing"].includes(
      transaction.stage ?? "",
    )
  );
}

export function loadCommunityOnboardingTransaction(
  storage: Storage = localStorage,
): CommunityOnboardingTransaction | null {
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    return isTransaction(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

export function saveCommunityOnboardingTransaction(
  transaction: CommunityOnboardingTransaction,
  storage: Storage = localStorage,
): void {
  if (typeof localStorage !== "undefined" && storage === localStorage) {
    setLocalStorageItemWithRecovery(STORAGE_KEY, JSON.stringify(transaction));
  } else {
    storage.setItem(STORAGE_KEY, JSON.stringify(transaction));
  }
}

export function clearCommunityOnboardingTransaction(
  storage: Storage = localStorage,
): void {
  storage.removeItem(STORAGE_KEY);
}

export function startCommunityOnboarding(
  input: StartCommunityOnboardingInput,
  storage: Storage = localStorage,
  now = new Date(),
): CommunityOnboardingTransaction {
  const relayUrl = canonicalRelayUrl(input.relayUrl);
  const existing = loadCommunityOnboardingTransaction(storage);
  if (existing?.relayUrl === relayUrl) {
    const updated = {
      ...existing,
      inviteCode: input.inviteCode?.trim() || existing.inviteCode,
      communityName: input.communityName?.trim() || existing.communityName,
      token: input.token?.trim() || existing.token,
      reposDir: input.reposDir ?? existing.reposDir,
      policyReceipt: input.policyReceipt ?? existing.policyReceipt,
      updatedAt: now.toISOString(),
      error: undefined,
      // A freshly opened link deserves fresh feedback — re-present the gate
      // even if a previous link for this relay was already dismissed.
      acknowledged: undefined,
    };
    saveCommunityOnboardingTransaction(updated, storage);
    return updated;
  }

  const timestamp = now.toISOString();
  const transaction: CommunityOnboardingTransaction = {
    id: crypto.randomUUID(),
    source: input.source,
    stage: input.inviteCode?.trim() ? "claiming" : "connecting",
    relayUrl,
    inviteCode: input.inviteCode?.trim() || undefined,
    communityName: input.communityName?.trim() || deriveCommunityName(relayUrl),
    token: input.token?.trim() || undefined,
    reposDir: input.reposDir,
    policyReceipt: input.policyReceipt,
    createdAt: timestamp,
    updatedAt: timestamp,
  };
  saveCommunityOnboardingTransaction(transaction, storage);
  return transaction;
}

export function updateCommunityOnboardingTransaction(
  transaction: CommunityOnboardingTransaction,
  patch: CommunityOnboardingTransactionPatch,
  storage: Storage = localStorage,
  now = new Date(),
): CommunityOnboardingTransaction {
  const updated = { ...transaction, ...patch, updatedAt: now.toISOString() };
  saveCommunityOnboardingTransaction(updated, storage);
  return updated;
}

export function updateCurrentCommunityOnboardingTransaction(
  current: CommunityOnboardingTransaction | null,
  patch: CommunityOnboardingTransactionPatch,
  expectedId: string | undefined,
  storage: Storage = localStorage,
  now = new Date(),
): CommunityOnboardingTransaction | null {
  if (!current || (expectedId && current.id !== expectedId)) return current;
  return updateCommunityOnboardingTransaction(current, patch, storage, now);
}

export function markCommunityOnboardingComplete(
  pubkey: string,
  relayUrl: string,
  storage: Storage = localStorage,
): void {
  storage.setItem(
    `buzz-community-onboarding-complete.v1:${encodeURIComponent(relayUrl)}:${pubkey}`,
    "true",
  );
  // The legacy gate is identity-scoped. Marking it here prevents the old profile
  // flow from reopening after the first community transaction completes.
  storage.setItem(`buzz-onboarding-complete.v1:${pubkey}`, "true");
}

import * as React from "react";

type CommunityOnboardingContextValue = {
  transaction: CommunityOnboardingTransaction | null;
  start: (input: StartCommunityOnboardingInput) => boolean;
  update: (
    patch: CommunityOnboardingTransactionPatch,
    expectedId?: string,
  ) => void;
  clear: () => void;
};

const CommunityOnboardingContext =
  React.createContext<CommunityOnboardingContextValue | null>(null);

export function CommunityOnboardingProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const [transaction, setTransaction] = React.useState(
    loadCommunityOnboardingTransaction,
  );
  const start = React.useCallback(
    (input: StartCommunityOnboardingInput) => {
      if (
        transaction &&
        canonicalRelayUrl(input.relayUrl) !== transaction.relayUrl
      ) {
        return false;
      }
      setTransaction(startCommunityOnboarding(input));
      return true;
    },
    [transaction],
  );
  const update = React.useCallback(
    (patch: CommunityOnboardingTransactionPatch, expectedId?: string) => {
      setTransaction((current) =>
        updateCurrentCommunityOnboardingTransaction(current, patch, expectedId),
      );
    },
    [],
  );
  const clear = React.useCallback(() => {
    clearCommunityOnboardingTransaction();
    setTransaction(null);
  }, []);
  const value = React.useMemo(
    () => ({ transaction, start, update, clear }),
    [clear, start, transaction, update],
  );
  return (
    <CommunityOnboardingContext.Provider value={value}>
      {children}
    </CommunityOnboardingContext.Provider>
  );
}

export function useCommunityOnboarding() {
  const context = React.useContext(CommunityOnboardingContext);
  if (!context)
    throw new Error(
      "useCommunityOnboarding must be used within CommunityOnboardingProvider",
    );
  return context;
}
