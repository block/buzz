import { invokeTauri } from "./tauri";

// ── Per-turn model routing ────────────────────────────────────────────────────
//
// The harness reads its policy from the file named by `BUZZ_ROUTING_POLICY`
// (crates/buzz-acp/src/routing.rs). These commands own the file; pointing the
// env var at the returned `path` is the caller's job, because the edit dialog
// replaces the whole env map on submit and would overwrite a backend patch.

export type RoutingMatchKind = "contains" | "contains_all";

export type RoutingRule = {
  name?: string | null;
  matchKind: RoutingMatchKind;
  /** Needles to look for. An empty list is rejected at save time. */
  any: string[];
  model: string;
};

export type RoutingPolicy = {
  enabled: boolean;
  rules: RoutingRule[];
  defaultModel?: string | null;
  /**
   * The optional local-Ollama classifier stage. Buzz has no UI for it, so it is
   * carried through opaquely — saving from the rules table must not silently
   * delete a classifier the user wrote into the file by hand.
   */
  classifier?: unknown;
};

export type AgentRoutingPolicyFile = {
  /** Where the policy lives — set `BUZZ_ROUTING_POLICY` to this. */
  path: string;
  /** `null` when nothing has been written yet. */
  policy: RoutingPolicy | null;
};

/**
 * Wire shape. The Rust side mirrors `buzz_acp::routing::Policy` verbatim, which
 * is snake_case, so these fields are NOT camelCase like the rest of our API —
 * the file has to be readable by the harness, not by us.
 */
type RawRoutingPolicy = {
  enabled: boolean;
  rules: {
    name?: string | null;
    match_kind: RoutingMatchKind;
    any: string[];
    model: string;
  }[];
  default_model?: string | null;
  classifier?: unknown;
};

type RawAgentRoutingPolicyFile = {
  path: string;
  policy: RawRoutingPolicy | null;
};

function fromRawRoutingPolicy(raw: RawRoutingPolicy): RoutingPolicy {
  return {
    enabled: raw.enabled,
    rules: (raw.rules ?? []).map((rule) => ({
      name: rule.name ?? null,
      matchKind: rule.match_kind ?? "contains",
      any: rule.any ?? [],
      model: rule.model,
    })),
    defaultModel: raw.default_model ?? null,
  };
}

function toRawRoutingPolicy(policy: RoutingPolicy): RawRoutingPolicy {
  return {
    enabled: policy.enabled,
    rules: policy.rules.map((rule) => ({
      name: rule.name?.trim() ? rule.name.trim() : null,
      match_kind: rule.matchKind,
      any: rule.any,
      model: rule.model,
    })),
    default_model: policy.defaultModel?.trim()
      ? policy.defaultModel.trim()
      : null,
  };
}

function fromRawRoutingPolicyFile(
  raw: RawAgentRoutingPolicyFile,
): AgentRoutingPolicyFile {
  return {
    path: raw.path,
    policy: raw.policy ? fromRawRoutingPolicy(raw.policy) : null,
  };
}

export async function getAgentRoutingPolicy(
  pubkey: string,
): Promise<AgentRoutingPolicyFile> {
  return fromRawRoutingPolicyFile(
    await invokeTauri<RawAgentRoutingPolicyFile>("get_agent_routing_policy", {
      pubkey,
    }),
  );
}

/** Pass `null` to delete the policy file. */
export async function setAgentRoutingPolicy(
  pubkey: string,
  policy: RoutingPolicy | null,
): Promise<AgentRoutingPolicyFile> {
  return fromRawRoutingPolicyFile(
    await invokeTauri<RawAgentRoutingPolicyFile>("set_agent_routing_policy", {
      pubkey,
      policy: policy ? toRawRoutingPolicy(policy) : null,
    }),
  );
}
