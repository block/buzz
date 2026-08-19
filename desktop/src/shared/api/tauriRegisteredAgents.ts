import { normalizePubkey } from "@/shared/lib/pubkey";
import { invokeTauri } from "./tauri";

export type RegisteredAgentReference = {
  pubkey: string;
  label: string | null;
  roleSummary: string | null;
  createdAt: string;
  updatedAt: string;
};

export type RawRegisteredAgentReference = {
  pubkey: string;
  label?: string | null;
  role_summary?: string | null;
  created_at: string;
  updated_at: string;
};

type RegisterExistingAgentInput = {
  pubkey: string;
  label?: string | null;
  roleSummary?: string | null;
};

export type RawRegisterExistingAgentInput = {
  pubkey: string;
  label: string | null;
  roleSummary: string | null;
};

const HEX_PUBKEY_RE = /^[0-9a-f]{64}$/;

function requiredString(value: unknown, field: string): string {
  if (typeof value !== "string") {
    throw new Error(`Malformed registered agent ${field}.`);
  }
  return value;
}

function nullableString(value: unknown, field: string): string | null {
  if (value === undefined || value === null) return null;
  if (typeof value !== "string") {
    throw new Error(`Malformed registered agent ${field}.`);
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function normalizeRegisteredPubkey(value: unknown): string {
  const pubkey = normalizePubkey(requiredString(value, "pubkey"));
  if (!HEX_PUBKEY_RE.test(pubkey)) {
    throw new Error("invalid public key");
  }
  return pubkey;
}

export function fromRawRegisteredAgentReference(
  raw: RawRegisteredAgentReference,
): RegisteredAgentReference {
  return {
    pubkey: normalizeRegisteredPubkey(raw.pubkey),
    label: nullableString(raw.label, "label"),
    roleSummary: nullableString(raw.role_summary, "role_summary"),
    createdAt: requiredString(raw.created_at, "created_at"),
    updatedAt: requiredString(raw.updated_at, "updated_at"),
  };
}

export function toRawRegisterExistingAgentInput(
  input: RegisterExistingAgentInput,
): RawRegisterExistingAgentInput {
  return {
    pubkey: normalizeRegisteredPubkey(input.pubkey),
    label: nullableString(input.label, "label"),
    roleSummary: nullableString(input.roleSummary, "roleSummary"),
  };
}

export async function listRegisteredAgentReferences(): Promise<
  RegisteredAgentReference[]
> {
  const raw = await invokeTauri<RawRegisteredAgentReference[]>(
    "list_registered_agent_references",
  );
  return raw.map(fromRawRegisteredAgentReference);
}

export async function registerExistingAgentReference(
  input: RegisterExistingAgentInput,
): Promise<RegisteredAgentReference> {
  const raw = await invokeTauri<RawRegisteredAgentReference>(
    "register_existing_agent_reference",
    { input: toRawRegisterExistingAgentInput(input) },
  );
  return fromRawRegisteredAgentReference(raw);
}

export async function unregisterExistingAgentReference(
  pubkey: string,
): Promise<void> {
  await invokeTauri("unregister_existing_agent_reference", {
    pubkey: normalizeRegisteredPubkey(pubkey),
  });
}
