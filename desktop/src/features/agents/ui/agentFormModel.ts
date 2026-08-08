/**
 * agentFormModel.ts — Canonical field model for the merged agent edit surface.
 *
 * One typed AgentFormModel is consumed by every edit route (Phase 1). The
 * create path extends the same model in Phase 2.
 *
 * Sections per Artifact 4:
 *   Identity        — name, avatar
 *   Behavior        — prompt, respond-to+allowlist, parallelism
 *   Runtime         — harness, model, provider, env vars, tuning
 *   Device policy   — auto-restart, start-on-launch (local backend only)
 *   Definition admin— name pool, share action, and definition-level actions
 *
 * Field ownership tags per Artifact 1:
 *   definition     — D-field: shared across linked siblings via definition
 *   instance       — I-field: this agent only
 *   local-policy   — L-field: dedicated setter, never on the relay
 *   create-only    — C-field: expressible at create time only
 *
 * Editability predicate:
 *   team-managed D-fields → read-only (D-fields render read-only)
 *   built-in → fully editable D-fields (only actions differ, not field editability)
 *   unlinked agent → definition sections absent
 *   zero-instance definition → instance + policy sections absent
 *   identity-archived agent → fully editable, Save never unarchives
 */

import type {
  AgentPersona,
  ManagedAgent,
  UpdatePersonaInput,
  UpdateManagedAgentInput,
} from "@/shared/api/types";

// ── Field ownership ─────────────────────────────────────────────────────────

export type FieldOwner = "definition" | "instance" | "local-policy";

// ── Context shape that drives editability ───────────────────────────────────

/**
 * Describes which combination of definition / instance is being edited.
 *
 * - definition-only: zero-instance definition (from agents library)
 * - instance-with-definition: linked agent (most common case, R1–R7)
 * - instance-only: unlinked agent (no definition)
 */
export type AgentEditContext =
  | { kind: "definition-only"; definition: AgentPersona }
  | {
      kind: "instance-with-definition";
      definition: AgentPersona;
      instance: ManagedAgent;
    }
  | { kind: "instance-only"; instance: ManagedAgent };

export function editContextDefinition(
  ctx: AgentEditContext,
): AgentPersona | null {
  if (ctx.kind === "instance-only") return null;
  return ctx.definition;
}

export function editContextInstance(
  ctx: AgentEditContext,
): ManagedAgent | null {
  if (ctx.kind === "definition-only") return null;
  return ctx.instance;
}

/** True when D-fields should render read-only (team-managed). */
export function isDefinitionReadOnly(ctx: AgentEditContext): boolean {
  const def = editContextDefinition(ctx);
  return def?.sourceTeam != null && def.sourceTeam.length > 0;
}

/** True when definition sections should be rendered at all. */
export function hasDefinitionContext(ctx: AgentEditContext): boolean {
  return ctx.kind !== "instance-only";
}

/** True when instance+policy sections should be rendered. */
export function hasInstanceContext(ctx: AgentEditContext): boolean {
  return ctx.kind !== "definition-only";
}

// ── AgentFormModel ──────────────────────────────────────────────────────────

/**
 * Flat, typed field model covering every editable field on the merged surface.
 *
 * Fields absent for a given context are undefined. Editability is determined
 * by AgentEditContext + the isDefinitionReadOnly predicate, NOT by AgentFormModel
 * itself — the model just holds current values.
 */
export type AgentFormModel = {
  // Identity (D when linked, I when unlinked)
  displayName: string;
  avatarUrl: string;

  // Behavior (D when linked, I when unlinked)
  systemPrompt: string;
  respondTo: ManagedAgent["respondTo"] | AgentPersona["respondTo"] | null;
  respondToAllowlist: string[];

  // Runtime (D when linked, I when unlinked) — harness pins are I
  /** Preferred runtime id from the definition. */
  runtime: string | undefined;
  model: string | null;
  provider: string | null;
  envVars: Record<string, string>;

  // Definition-only fields
  /** Name pool for definition. Undefined when no definition context. */
  namePool: string[] | undefined;

  // Instance-level fields
  /** Instance name (per-pool override of displayName). */
  instanceName: string | undefined;
  /**
   * Instance-level env-var overlay (row 8 contract). When a definition is
   * present the definition env is inherited; this field holds the per-instance
   * additions/overrides. Undefined when no instance context.
   */
  instanceEnvVars: Record<string, string> | undefined;
  parallelism: number | null;

  // Device policy (L-fields)
  autoRestartOnConfigChange: boolean | undefined;
  startOnAppLaunch: boolean | undefined;
};

/** Which coordinator outputs changed vs. last saved state. */
export type AgentFormEmit = {
  /** D-field update payload, present iff a D-field changed AND the definition is editable. */
  personaInput: UpdatePersonaInput | null;
  /** I-field update payload, present iff an I-field changed. */
  agentInput: UpdateManagedAgentInput | null;
  /** Policy-setter calls, one entry per changed L-field. */
  policySets: Array<
    | { type: "autoRestart"; pubkey: string; value: boolean }
    | { type: "startOnAppLaunch"; pubkey: string; value: boolean }
  >;
};

// ── Canonical comparison helpers ─────────────────────────────────────────────

/** Map equality per Artifact 3 settlement contract. */
export function envVarsMapEqual(
  a: Record<string, string>,
  b: Record<string, string>,
): boolean {
  const aKeys = Object.keys(a);
  const bKeys = Object.keys(b);
  if (aKeys.length !== bKeys.length) return false;
  return aKeys.every((k) => a[k] === b[k]);
}

/** Canonical name-pool equality: order-sensitive, trimmed. */
export function namePoolEqual(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((v, i) => v.trim() === b[i]?.trim());
}

// ── Edit adapters ────────────────────────────────────────────────────────────

/**
 * Seed an AgentFormModel from (definition?, agent?) — at least one present.
 *
 * For instance-with-definition: D-fields seed from the definition (authoritative
 * source); I-fields seed from the instance. For definition-only: instance fields
 * are undefined. For instance-only: definition fields are undefined.
 */
export function seedAgentFormModel(ctx: AgentEditContext): AgentFormModel {
  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);

  // D-fields: from definition when present, else from instance
  const displayName = def?.displayName ?? inst?.name ?? "";
  const avatarUrl = def?.avatarUrl ?? inst?.avatarUrl ?? "";
  const systemPrompt = def?.systemPrompt ?? inst?.systemPrompt ?? "";
  const respondTo = def?.respondTo ?? inst?.respondTo ?? null;
  const respondToAllowlist =
    def?.respondToAllowlist ?? inst?.respondToAllowlist ?? [];
  const runtime = def?.runtime ?? undefined;
  const model = def !== null ? (def.model ?? null) : (inst?.model ?? null);
  const provider =
    def !== null ? (def.provider ?? null) : (inst?.provider ?? null);
  // Env: D-field base. Instance overlay is applied separately at spawn;
  // the form edits each store independently (row 8 contract).
  const envVars = def?.envVars ?? inst?.envVars ?? {};
  const namePool = def?.namePool;

  // I-fields
  const instanceName = inst?.name;
  // Row 8 contract: when a definition is present, instance env is a per-agent
  // overlay that does NOT wholesale-replace the definition env. Seed from
  // inst.envVars directly (not the merged/inherited snapshot).
  const instanceEnvVars = inst != null ? (inst.envVars ?? {}) : undefined;
  const parallelism = inst?.parallelism ?? def?.parallelism ?? null;

  // L-fields
  const autoRestartOnConfigChange = inst?.autoRestartOnConfigChange;
  const startOnAppLaunch = inst?.startOnAppLaunch;

  return {
    displayName,
    avatarUrl,
    systemPrompt,
    respondTo,
    respondToAllowlist,
    runtime,
    model,
    provider,
    envVars,
    namePool,
    instanceName,
    instanceEnvVars,
    parallelism,
    autoRestartOnConfigChange,
    startOnAppLaunch,
  };
}

/**
 * Emit coordinator inputs from a (previous model / saved state) pair.
 *
 * Returns personaInput when a D-field changed AND the definition is editable.
 * Returns agentInput when an I-field changed.
 * Returns policySets for any L-field changed.
 *
 * The caller (save coordinator) calls this after re-fetching observed state,
 * so "previous" is the re-fetched stored state and "next" is what the user
 * submitted — the diff is exactly what hasn't been persisted yet.
 */
export function emitAgentFormDiff(
  saved: AgentFormModel,
  next: AgentFormModel,
  ctx: AgentEditContext,
  _options?: { publishCatalogUpdates?: boolean },
): AgentFormEmit {
  const def = editContextDefinition(ctx);
  const inst = editContextInstance(ctx);
  const defReadOnly = isDefinitionReadOnly(ctx);

  // D-field diff — only when definition is present and NOT team-managed
  let personaInput: UpdatePersonaInput | null = null;
  if (def !== null && !defReadOnly) {
    // respondTo is a D-field when definition is present: changes are written
    // to the definition, not to the instance. Include it in the D-diff check.
    const respondToChanged =
      (next.respondTo ?? null) !== (saved.respondTo ?? null);
    const allowlistChanged =
      next.respondTo === "allowlist" &&
      next.respondToAllowlist.join(",") !==
        (saved.respondToAllowlist ?? []).join(",");

    const dChanged =
      next.displayName.trim() !== saved.displayName.trim() ||
      (next.avatarUrl ?? "") !== (saved.avatarUrl ?? "") ||
      next.systemPrompt.trim() !== saved.systemPrompt.trim() ||
      next.runtime !== saved.runtime ||
      (next.model ?? null) !== (saved.model ?? null) ||
      (next.provider ?? null) !== (saved.provider ?? null) ||
      !envVarsMapEqual(next.envVars ?? {}, saved.envVars ?? {}) ||
      !namePoolEqual(next.namePool ?? [], saved.namePool ?? []) ||
      respondToChanged ||
      allowlistChanged;

    if (dChanged) {
      personaInput = {
        id: def.id,
        displayName: next.displayName.trim(),
        avatarUrl: next.avatarUrl ?? "",
        systemPrompt: next.systemPrompt.trim(),
        runtime: next.runtime,
        model: next.model ?? undefined,
        provider: next.provider ?? undefined,
        namePool: next.namePool ?? [],
        envVars: next.envVars ?? {},
        // Always emit behavior block when definition is present — respondTo
        // is a D-field and must travel with the persona write.
        behavior:
          next.respondTo != null
            ? {
                respondTo: next.respondTo,
                respondToAllowlist:
                  next.respondTo === "allowlist"
                    ? next.respondToAllowlist
                    : undefined,
                parallelism: next.parallelism ?? undefined,
              }
            : undefined,
      };
    }
  }

  // I-field diff — only when instance is present
  let agentInput: UpdateManagedAgentInput | null = null;
  if (inst !== null) {
    // Row 1 contract: stop materializing displayName→name when a name pool exists
    const hasNamePool = (def?.namePool?.length ?? 0) > 0;

    // Row 8 contract: instance env edit goes to instance, NOT wholesale-replace from definition.
    // When a definition is present, instanceEnvVars holds the per-instance overlay.
    const instanceEnvChanged =
      next.instanceEnvVars !== undefined
        ? !envVarsMapEqual(next.instanceEnvVars ?? {}, inst.envVars ?? {})
        : false;

    const nameChanged =
      !hasNamePool &&
      (next.instanceName ?? next.displayName.trim()) !== inst.name;
    const systemPromptChanged =
      def === null &&
      (next.systemPrompt.trim() || null) !== (inst.systemPrompt ?? null);
    const modelChanged =
      def === null && (next.model ?? null) !== (inst.model ?? null);
    const providerChanged =
      def === null && (next.provider ?? null) !== (inst.provider ?? null);
    // respondTo is a D-field when a definition is present AND editable.
    // When the definition is team-managed (read-only) or absent, changes fall
    // through to the instance so they are never silently dropped.
    const respondToInstanceOwned = def === null || defReadOnly;
    const respondToChanged =
      respondToInstanceOwned &&
      (next.respondTo ?? null) !== (inst.respondTo ?? null);
    const allowlistChanged =
      respondToInstanceOwned &&
      next.respondTo === "allowlist" &&
      next.respondToAllowlist.join(",") !== inst.respondToAllowlist.join(",");
    const parallelismChanged =
      (next.parallelism ?? null) !== (inst.parallelism ?? null);

    const iChanged =
      nameChanged ||
      systemPromptChanged ||
      modelChanged ||
      providerChanged ||
      instanceEnvChanged ||
      respondToChanged ||
      allowlistChanged ||
      parallelismChanged;

    if (iChanged) {
      agentInput = { pubkey: inst.pubkey };
      if (nameChanged)
        agentInput.name = next.instanceName ?? next.displayName.trim();
      if (systemPromptChanged)
        agentInput.systemPrompt = next.systemPrompt.trim() || null;
      if (modelChanged) agentInput.model = next.model ?? null;
      if (providerChanged) agentInput.provider = next.provider ?? null;
      if (instanceEnvChanged) agentInput.envVars = next.instanceEnvVars ?? {};
      if (respondToChanged) agentInput.respondTo = next.respondTo ?? undefined;
      if (allowlistChanged)
        agentInput.respondToAllowlist = next.respondToAllowlist;
      if (parallelismChanged && next.parallelism != null)
        agentInput.parallelism = next.parallelism;
    }
  }

  // L-field diff
  const policySets: AgentFormEmit["policySets"] = [];
  if (inst !== null) {
    if (
      next.autoRestartOnConfigChange !== undefined &&
      next.autoRestartOnConfigChange !== saved.autoRestartOnConfigChange
    ) {
      policySets.push({
        type: "autoRestart",
        pubkey: inst.pubkey,
        value: next.autoRestartOnConfigChange,
      });
    }
    if (
      next.startOnAppLaunch !== undefined &&
      next.startOnAppLaunch !== saved.startOnAppLaunch
    ) {
      policySets.push({
        type: "startOnAppLaunch",
        pubkey: inst.pubkey,
        value: next.startOnAppLaunch,
      });
    }
  }

  return { personaInput, agentInput, policySets };
}
